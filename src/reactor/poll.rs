use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use std::{
    collections::{BTreeMap, VecDeque},
    os::fd::{AsFd, AsRawFd},
    sync::{Arc, Mutex},
};

use super::WakeupKind;

struct RwQueue<T: Send + Sync> {
    readers: VecDeque<T>,
    writers: VecDeque<T>,
}

impl<T: Send + Sync> RwQueue<T> {
    fn insert(&mut self, rw: WakeupKind, obj: T) {
        match rw {
            WakeupKind::Readable => self.readers.push_back(obj),
            WakeupKind::Writable => self.writers.push_back(obj),
        }
    }

    fn is_empty(&self) -> bool {
        self.readers.is_empty() && self.writers.is_empty()
    }

    fn get(&mut self, flag: EpollFlags) -> Option<T> {
        match flag {
            EpollFlags::EPOLLIN => self.readers.pop_front(),
            EpollFlags::EPOLLOUT => self.writers.pop_front(),
            _ => self.readers.pop_front().or(self.writers.pop_front()),
        }
    }

    fn new() -> Self {
        Self {
            readers: VecDeque::new(),
            writers: VecDeque::new(),
        }
    }
}

pub struct Subscription<T: Send + Sync> {
    fd: Arc<dyn AsFd + Send + Sync>,
    queue: RwQueue<T>,
}

pub struct Poll<T: Send + Sync> {
    subscriptions: Arc<Mutex<BTreeMap<i32, Subscription<T>>>>,
    epoll: Epoll,
}

impl<T: Send + Sync> Poll<T> {
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            epoll: Epoll::new(EpollCreateFlags::empty()).unwrap(),
        }
    }

    pub fn insert(&self, fd: Arc<dyn AsFd + Send + Sync>, data: T, kind: WakeupKind) {
        let mut subscriptions = self.subscriptions.lock().unwrap();
        let idx = fd.as_fd().as_raw_fd();
        if let Some(entry) = subscriptions.get_mut(&idx) {
            entry.queue.insert(kind, data);
        } else {
            let mut sub = Subscription {
                fd: fd.clone(),
                queue: RwQueue::new(),
            };

            sub.queue.insert(kind, data);

            let epoll_event =
                EpollEvent::new(EpollFlags::EPOLLIN | EpollFlags::EPOLLOUT, idx as u64);
            self.epoll.add(fd.as_fd(), epoll_event).unwrap();

            subscriptions.insert(idx, sub);
        }
    }

    pub fn wait(&self) -> T {
        loop {
            let mut event = [EpollEvent::empty()];
            let n = self.epoll.wait(&mut event, EpollTimeout::NONE).unwrap();

            assert_eq!(n, 1);

            {
                let idx = event[0].data() as i32;
                let mut subscriptions = self.subscriptions.lock().unwrap();

                if let Some(sub) = subscriptions.get_mut(&idx) {
                    if let Some(ret) = sub.queue.get(event[0].events()) {
                        if sub.queue.is_empty() {
                            let sub = subscriptions.remove(&idx).unwrap();
                            self.epoll.delete(sub.fd.as_fd()).unwrap();
                        }
                        return ret;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{os::fd::OwnedFd, sync::Arc, thread::sleep, time::Duration};

    use nix::unistd::{pipe, write};

    use crate::reactor::WakeupKind;

    use super::Poll;

    fn setup_test() -> (Arc<OwnedFd>, Arc<OwnedFd>, Poll<u32>) {
        let (fd_rd, fd_wr) = pipe().unwrap();
        let fd_rd = Arc::new(fd_rd);
        let fd_wr = Arc::new(fd_wr);
        let poll: Poll<u32> = Poll::new();

        (fd_rd, fd_wr, poll)
    }

    #[test]
    fn single_wakeup_read() {
        let (fd_rd, fd_wr, poll) = setup_test();

        poll.insert(fd_rd.clone(), 1, WakeupKind::Readable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), 1);

            assert!(poll.subscriptions.lock().unwrap().len() == 0);
        });

        write(fd_wr, &[2]).unwrap();

        t1.join().unwrap();
    }

    #[test]
    fn single_wakeup_write() {
        let (_fd_rd, fd_wr, poll) = setup_test();

        poll.insert(fd_wr.clone(), 1, WakeupKind::Writable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), 1);
            write(fd_wr, &[2]).unwrap();
        });

        t1.join().unwrap();
    }

    #[test]
    fn multi_wakeup_same_fd_read() {
        let (fd_rd, fd_wr, poll) = setup_test();

        poll.insert(fd_rd.clone(), 1, WakeupKind::Readable);
        poll.insert(fd_rd.clone(), 2, WakeupKind::Readable);
        poll.insert(fd_rd.clone(), 3, WakeupKind::Readable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), 1);
            assert!(poll.subscriptions.lock().unwrap().len() == 1);
            assert_eq!(poll.wait(), 2);
            assert!(poll.subscriptions.lock().unwrap().len() == 1);
            assert_eq!(poll.wait(), 3);
            assert!(poll.subscriptions.lock().unwrap().len() == 0);
        });

        write(fd_wr, &[2]).unwrap();

        t1.join().unwrap();
    }

    #[test]
    fn multi_wakeup_dual_fds_read() {
        let (fd_rd, fd_wr, poll) = setup_test();
        let (fd2_rd, fd2_wr, _) = setup_test();

        poll.insert(fd_rd.clone(), 1, WakeupKind::Readable);
        poll.insert(fd_rd.clone(), 2, WakeupKind::Readable);
        poll.insert(fd_rd.clone(), 3, WakeupKind::Readable);
        poll.insert(fd2_rd.clone(), 4, WakeupKind::Readable);
        poll.insert(fd2_rd.clone(), 5, WakeupKind::Readable);
        poll.insert(fd2_rd.clone(), 6, WakeupKind::Readable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), 4);
            assert!(poll.subscriptions.lock().unwrap().len() == 2);
            assert_eq!(poll.wait(), 5);
            assert!(poll.subscriptions.lock().unwrap().len() == 2);
            assert_eq!(poll.wait(), 6);
            assert!(poll.subscriptions.lock().unwrap().len() == 1);
            assert_eq!(poll.wait(), 1);
            assert!(poll.subscriptions.lock().unwrap().len() == 1);
            assert_eq!(poll.wait(), 2);
            assert!(poll.subscriptions.lock().unwrap().len() == 1);
            assert_eq!(poll.wait(), 3);
            assert!(poll.subscriptions.lock().unwrap().len() == 0);
        });

        write(fd2_wr, &[2]).unwrap();

        sleep(Duration::from_secs(1));

        write(fd_wr, &[2]).unwrap();

        t1.join().unwrap();
    }
}
