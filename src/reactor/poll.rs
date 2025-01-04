use libc::{epoll_event, EPOLLIN, EPOLLOUT, EPOLL_CTL_ADD, EPOLL_CTL_DEL};
use log::debug;
use std::{
    collections::{BTreeMap, VecDeque},
    mem::MaybeUninit,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    ptr::null_mut,
    sync::{Arc, Mutex},
};

use super::WakeupKind;

struct RwQueue<T> {
    readers: VecDeque<T>,
    writers: VecDeque<T>,
}

impl<T> RwQueue<T> {
    fn insert(&mut self, rw: WakeupKind, obj: T) {
        match rw {
            WakeupKind::Readable => self.readers.push_back(obj),
            WakeupKind::Writable => self.writers.push_back(obj),
        }
    }

    fn is_empty(&self) -> bool {
        self.readers.is_empty() && self.writers.is_empty()
    }

    fn get(&mut self, flag: u32) -> Option<T> {
        if flag & EPOLLIN as u32 != 0 {
            self.readers.pop_front()
        } else {
            self.writers.pop_front()
        }
    }

    fn new() -> Self {
        Self {
            readers: VecDeque::new(),
            writers: VecDeque::new(),
        }
    }
}

pub struct Subscription<T> {
    fd: i32,
    queue: RwQueue<T>,
}

impl<T> From<&Subscription<T>> for libc::epoll_event {
    fn from(value: &Subscription<T>) -> Self {
        let mut events = 0;

        if !value.queue.readers.is_empty() {
            events |= EPOLLIN;
        }

        if !value.queue.writers.is_empty() {
            events |= EPOLLOUT;
        }

        libc::epoll_event {
            events: events as u32,
            u64: value.fd as u64,
        }
    }
}

pub struct Poll<T> {
    subscriptions: Arc<Mutex<BTreeMap<i32, Subscription<T>>>>,
    epoll: OwnedFd,
}

impl<T> Poll<T> {
    pub fn new() -> std::io::Result<Self> {
        let epoll = unsafe { libc::epoll_create1(0) };

        if epoll == -1 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            epoll: unsafe { OwnedFd::from_raw_fd(epoll) },
        })
    }

    pub fn insert(&self, fd: RawFd, data: T, kind: WakeupKind) {
        debug!("Inserting FD {fd:} for waking up kind: {kind:?}");
        let mut subscriptions = self.subscriptions.lock().unwrap();
        if let Some(entry) = subscriptions.get_mut(&fd) {
            entry.queue.insert(kind, data);
        } else {
            let mut sub = Subscription {
                fd,
                queue: RwQueue::new(),
            };

            sub.queue.insert(kind, data);

            let mut epoll_event: libc::epoll_event = (&sub).into();

            let ret = unsafe {
                libc::epoll_ctl(
                    self.epoll.as_raw_fd(),
                    EPOLL_CTL_ADD,
                    fd,
                    &mut epoll_event as *mut epoll_event,
                )
            };

            if ret == -1 {
                panic!(
                    "Could not ad FD to epoll {}",
                    std::io::Error::last_os_error()
                );
            }

            subscriptions.insert(fd, sub);
        }
    }

    pub fn wait(&self) -> T {
        loop {
            let event = unsafe {
                let mut event: MaybeUninit<libc::epoll_event> = MaybeUninit::uninit();
                debug!("Calling epoll_wait");
                let n = libc::epoll_wait(self.epoll.as_raw_fd(), event.as_mut_ptr(), 1, -1);
                if n != 1 {
                    panic!("epoll returned non-1 value");
                }

                event.assume_init()
            };

            {
                debug!("Epoll returned");
                let idx = event.u64 as i32;
                let mut subscriptions = self.subscriptions.lock().unwrap();

                if let Some(sub) = subscriptions.get_mut(&idx) {
                    if let Some(ret) = sub.queue.get(event.events) {
                        if sub.queue.is_empty() {
                            let sub = subscriptions.remove(&idx).unwrap();
                            let ret = unsafe {
                                libc::epoll_ctl(
                                    self.epoll.as_raw_fd(),
                                    EPOLL_CTL_DEL,
                                    sub.fd,
                                    null_mut(),
                                )
                            };
                            if ret == -1 {
                                panic!("Could not remove fd from epoll");
                            }
                        }
                        debug!("Returning event");
                        return ret;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        thread::sleep,
        time::Duration,
    };

    use libc::O_NONBLOCK;

    use crate::reactor::WakeupKind;

    use super::Poll;

    fn write(fd: impl AsFd, buf: &[u8]) {
        let ret = unsafe {
            libc::write(
                fd.as_fd().as_raw_fd(),
                buf.as_ptr() as *const _,
                buf.len() as _,
            )
        };

        if ret == -1 {
            panic!("write failed");
        }
    }

    fn setup_test() -> (OwnedFd, OwnedFd, Poll<u32>) {
        let mut fds = [0, 0];
        let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), O_NONBLOCK) };

        if ret == -1 {
            panic!("Pipe failed");
        }

        let fd_rd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let fd_wr = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        let poll: Poll<u32> = Poll::new().unwrap();

        (fd_rd, fd_wr, poll)
    }

    #[test]
    fn single_wakeup_read() {
        let (fd_rd, fd_wr, poll) = setup_test();

        poll.insert(fd_rd.as_raw_fd(), 1, WakeupKind::Readable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), 1);
            assert!(poll.subscriptions.lock().unwrap().len() == 0);
        });

        write(fd_wr, &[2]);

        t1.join().unwrap();
    }

    #[test]
    fn single_wakeup_write() {
        let (_fd_rd, fd_wr, poll) = setup_test();

        poll.insert(fd_wr.as_raw_fd(), 1, WakeupKind::Writable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), 1);
        });

        t1.join().unwrap();
    }

    #[test]
    fn multi_wakeup_same_fd_read() {
        let (fd_rd, fd_wr, poll) = setup_test();

        poll.insert(fd_rd.as_raw_fd(), 1, WakeupKind::Readable);
        poll.insert(fd_rd.as_raw_fd(), 2, WakeupKind::Readable);
        poll.insert(fd_rd.as_raw_fd(), 3, WakeupKind::Readable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), 1);
            assert!(poll.subscriptions.lock().unwrap().len() == 1);
            assert_eq!(poll.wait(), 2);
            assert!(poll.subscriptions.lock().unwrap().len() == 1);
            assert_eq!(poll.wait(), 3);
            assert!(poll.subscriptions.lock().unwrap().len() == 0);
        });

        write(fd_wr, &[2]);

        t1.join().unwrap();
    }

    #[test]
    fn multi_wakeup_dual_fds_read() {
        let (fd_rd, fd_wr, poll) = setup_test();
        let (fd2_rd, fd2_wr, _) = setup_test();

        poll.insert(fd_rd.as_raw_fd(), 1, WakeupKind::Readable);
        poll.insert(fd_rd.as_raw_fd(), 2, WakeupKind::Readable);
        poll.insert(fd_rd.as_raw_fd(), 3, WakeupKind::Readable);
        poll.insert(fd2_rd.as_raw_fd(), 4, WakeupKind::Readable);
        poll.insert(fd2_rd.as_raw_fd(), 5, WakeupKind::Readable);
        poll.insert(fd2_rd.as_raw_fd(), 6, WakeupKind::Readable);

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

        write(fd2_wr, &[2]);

        sleep(Duration::from_secs(1));

        write(fd_wr, &[2]);

        t1.join().unwrap();
    }
}
