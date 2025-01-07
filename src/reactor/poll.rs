use events::Events;
use libc::{EPOLLIN, EPOLLOUT, EPOLL_CTL_ADD, EPOLL_CTL_DEL, EPOLL_CTL_MOD};
use log::debug;
use std::{
    collections::BTreeMap,
    ffi::c_int,
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd},
    ptr::null_mut,
};

use super::WakeupKind;

mod events;

struct RwQueue<T> {
    readers: Vec<T>,
    writers: Vec<T>,
}

impl<T> RwQueue<T> {
    fn insert(&mut self, rw: WakeupKind, obj: T) {
        match rw {
            WakeupKind::Readable => self.readers.push(obj),
            WakeupKind::Writable => self.writers.push(obj),
        }
    }

    fn is_empty(&self) -> bool {
        self.readers.is_empty() && self.writers.is_empty()
    }

    fn get_objs_for_flags(&mut self, flag: u32) -> Vec<T> {
        let mut ret = Vec::new();

        if flag & EPOLLIN as u32 != 0 {
            ret.append(&mut self.readers);
        }

        if flag & EPOLLOUT as u32 != 0 {
            ret.append(&mut self.writers);
        }

        ret
    }

    fn get_flags(&self) -> Option<u32> {
        let mut ret = 0;

        if self.is_empty() {
            return None;
        }

        if !self.readers.is_empty() {
            ret |= EPOLLIN;
        }

        if !self.writers.is_empty() {
            ret |= EPOLLOUT;
        }

        Some(ret as u32)
    }

    fn new() -> Self {
        Self {
            readers: Vec::new(),
            writers: Vec::new(),
        }
    }
}

pub struct Subscription<T> {
    fd: i32,
    queue: RwQueue<T>,
}

impl<T> TryFrom<&Subscription<T>> for libc::epoll_event {
    type Error = ();

    fn try_from(value: &Subscription<T>) -> Result<Self, Self::Error> {
        let flags = value.queue.get_flags().ok_or(())?;

        Ok(libc::epoll_event {
            events: flags,
            u64: value.fd as u64,
        })
    }
}

pub struct Poll<T> {
    subscriptions: BTreeMap<i32, Subscription<T>>,
    epoll: OwnedFd,
}

impl<T> Poll<T> {
    pub fn new() -> std::io::Result<Self> {
        let epoll = unsafe { libc::epoll_create1(0) };

        if epoll == -1 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            subscriptions: BTreeMap::new(),
            epoll: unsafe { OwnedFd::from_raw_fd(epoll) },
        })
    }

    pub fn insert(&mut self, fd: RawFd, data: T, kind: WakeupKind) {
        debug!("Inserting FD {fd:} for waking up kind: {kind:?}");
        let sub = if let Some(mut sub) = self.subscriptions.remove(&fd) {
            sub.queue.insert(kind, data);

            let epoll_event = (&sub).try_into().unwrap();

            self.epoll_ctl(EPOLL_CTL_MOD, fd, Some(epoll_event))
                .unwrap();

            sub
        } else {
            let mut sub = Subscription {
                fd,
                queue: RwQueue::new(),
            };

            sub.queue.insert(kind, data);

            let epoll_event = (&sub).try_into().unwrap();

            self.epoll_ctl(EPOLL_CTL_ADD, fd, Some(epoll_event))
                .unwrap();

            sub
        };

        self.subscriptions.insert(fd, sub);
    }

    fn epoll_ctl(
        &self,
        op: c_int,
        fd: c_int,
        mut event: Option<libc::epoll_event>,
    ) -> std::io::Result<()> {
        let evt = event.as_mut().map(|x| x as *mut _).unwrap_or(null_mut());

        let ret = unsafe { libc::epoll_ctl(self.epoll.as_raw_fd(), op, fd, evt) };

        if ret == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn wait(&mut self) -> Vec<T> {
        let mut events = Events::new();
        let mut ret = Vec::new();

        loop {
            for evt in events.wait(self.epoll.as_fd()).unwrap() {
                let idx = evt.u64 as i32;

                if let Some((fd, mut sub)) = self.subscriptions.remove_entry(&idx) {
                    ret.append(&mut sub.queue.get_objs_for_flags(evt.events));

                    match (&sub).try_into() {
                        Ok(epoll_event) => {
                            self.epoll_ctl(EPOLL_CTL_MOD, sub.fd, Some(epoll_event))
                                .unwrap();
                            self.subscriptions.insert(fd, sub);
                        }
                        Err(()) => {
                            self.epoll_ctl(EPOLL_CTL_DEL, sub.fd, None).unwrap();
                        }
                    }
                }
            }

            if !ret.is_empty() {
                return ret;
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

    use libc::{AF_LOCAL, SOCK_NONBLOCK, SOCK_STREAM};

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
        let ret =
            unsafe { libc::socketpair(AF_LOCAL, SOCK_STREAM | SOCK_NONBLOCK, 0, fds.as_mut_ptr()) };

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
        let (fd_rd, fd_wr, mut poll) = setup_test();

        poll.insert(fd_rd.as_raw_fd(), 1, WakeupKind::Readable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait()[0], 1);
            assert!(poll.subscriptions.len() == 0);
        });

        write(fd_wr, &[2]);

        t1.join().unwrap();
    }

    #[test]
    fn single_wakeup_write() {
        let (_fd_rd, fd_wr, mut poll) = setup_test();

        poll.insert(fd_wr.as_raw_fd(), 1, WakeupKind::Writable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait()[0], 1);
        });

        t1.join().unwrap();
    }

    #[test]
    fn multi_wakeup_same_fd_read() {
        let (fd_rd, fd_wr, mut poll) = setup_test();

        poll.insert(fd_rd.as_raw_fd(), 1, WakeupKind::Readable);
        poll.insert(fd_rd.as_raw_fd(), 2, WakeupKind::Readable);
        poll.insert(fd_rd.as_raw_fd(), 3, WakeupKind::Readable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), [1, 2, 3]);
            assert!(poll.subscriptions.len() == 0);
        });

        write(fd_wr, &[2]);

        t1.join().unwrap();
    }

    #[test]
    fn multi_wakeup_dual_fds_read() {
        let (fd_rd, fd_wr, mut poll) = setup_test();
        let (fd2_rd, fd2_wr, _) = setup_test();

        poll.insert(fd_rd.as_raw_fd(), 1, WakeupKind::Readable);
        poll.insert(fd_rd.as_raw_fd(), 2, WakeupKind::Readable);
        poll.insert(fd_rd.as_raw_fd(), 3, WakeupKind::Readable);
        poll.insert(fd2_rd.as_raw_fd(), 4, WakeupKind::Readable);
        poll.insert(fd2_rd.as_raw_fd(), 5, WakeupKind::Readable);
        poll.insert(fd2_rd.as_raw_fd(), 6, WakeupKind::Readable);

        let t1 = std::thread::spawn(move || {
            assert_eq!(poll.wait(), [4, 5, 6]);
            assert!(poll.subscriptions.len() == 1);
            assert_eq!(poll.wait(), [1, 2, 3]);
            assert!(poll.subscriptions.len() == 0);
        });

        write(fd2_wr, &[2]);

        sleep(Duration::from_secs(1));

        write(fd_wr, &[2]);

        t1.join().unwrap();
    }

    #[test]
    fn sub_read_and_write() {
        let (fd1, fd2, mut poll) = setup_test();

        write(fd2.as_fd(), &[1]);

        poll.insert(fd1.as_raw_fd(), 1, WakeupKind::Readable);
        poll.insert(fd1.as_raw_fd(), 2, WakeupKind::Writable);
        poll.insert(fd2.as_raw_fd(), 3, WakeupKind::Writable);

        assert_eq!(poll.wait(), [1, 2, 3]);
        assert!(poll.subscriptions.len() == 0);
    }
}
