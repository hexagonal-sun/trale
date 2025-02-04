use std::cell::RefCell;

use io_uring::{squeue, IoUring};
use slab::Slab;

use super::io::{RingResults, UringIo};

pub struct ReactorUring<T>(RefCell<ReactorInner<T>>);

impl<T> ReactorUring<T> {
    pub fn new() -> Self {
        Self(RefCell::new(ReactorInner::new()))
    }

    pub fn new_io(&self) -> UringIo<'_, T> {
        UringIo::new(&self.0)
    }

    pub fn react(&self) -> Vec<T> {
        self.0.borrow_mut().react()
    }
}

pub(super) struct ReactorInner<T> {
    uring: IoUring,
    objs: Slab<(T, usize)>,
    pub results: RingResults,
}

impl<T> ReactorInner<T> {
    pub fn new() -> Self {
        Self {
            uring: IoUring::new(1024).unwrap(),
            objs: Slab::new(),
            results: RingResults::new(),
        }
    }

    pub fn submit_io(&mut self, entry: squeue::Entry, obj: T) -> usize {
        let result_idx = self.results.create_slot();

        let slot = self.objs.insert((obj, result_idx));

        unsafe {
            self.uring
                .submission()
                .push(&entry.user_data(slot as u64))
                .unwrap();
        }

        result_idx
    }

    pub fn react(&mut self) -> Vec<T> {
        self.uring.submit_and_wait(1).unwrap();
        let completions = unsafe { self.uring.completion_shared() };

        completions
            .map(|x| {
                let (obj, result_idx) = self.objs.remove(x.user_data() as usize);
                self.results.set_result(x.result(), result_idx);

                obj
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        task::Poll,
    };

    use io_uring::{opcode, types};
    use libc::{AF_LOCAL, SOCK_NONBLOCK, SOCK_STREAM};

    use super::ReactorUring;

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

    fn read(fd: impl AsFd, buf: &mut [u8]) {
        let ret = unsafe {
            libc::read(
                fd.as_fd().as_raw_fd(),
                buf.as_mut_ptr() as *mut _,
                buf.len() as _,
            )
        };

        if ret == -1 {
            panic!("write failed");
        }
    }

    fn run_test(f: impl FnOnce(OwnedFd, OwnedFd, &mut ReactorUring<u32>)) {
        let mut fds = [0, 0];
        let ret =
            unsafe { libc::socketpair(AF_LOCAL, SOCK_STREAM | SOCK_NONBLOCK, 0, fds.as_mut_ptr()) };

        if ret == -1 {
            panic!("Pipe failed");
        }

        let a = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let b = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        let mut uring = ReactorUring::new();

        f(a, b, &mut uring);

        assert_eq!(uring.0.borrow().results.0.len(), 0);
    }

    #[test]
    fn single_wakeup_read() {
        run_test(|a, b, uring| {
            let mut buf = [0];

            let mut io = uring.new_io();
            let result = io.submit_or_get_result(|| {
                (
                    opcode::Read::new(types::Fd(a.as_raw_fd()), buf.as_mut_ptr(), 1).build(),
                    10,
                )
            });

            assert!(matches!(result, Poll::Pending));

            let t1 = std::thread::spawn(move || {
                write(b, &[2]);
            });

            let objs = uring.react();

            assert_eq!(objs, vec![10]);

            let result =
                io.submit_or_get_result(|| panic!("Should not be called, as result will be ready"));

            assert!(matches!(result, Poll::Ready(Ok(1))));

            t1.join().unwrap();
        });
    }

    #[test]
    fn io_dropped_before_react_cleanup() {
        run_test(|a, b, uring| {
            let mut buf = [0];

            let mut io = uring.new_io();
            assert!(matches!(
                io.submit_or_get_result(|| {
                    (
                        opcode::Read::new(types::Fd(a.as_raw_fd()), buf.as_mut_ptr(), 1).build(),
                        10,
                    )
                }),
                Poll::Pending
            ));

            drop(io);

            let t1 = std::thread::spawn(move || {
                write(b, &[2]);
            });

            let objs = uring.react();

            assert_eq!(objs, vec![10]);

            t1.join().unwrap();
        });
    }

    #[test]
    fn single_wakeup_write() {
        run_test(|a, b, uring| {
            let buf = [0];

            let mut io = uring.new_io();
            let result = io.submit_or_get_result(|| {
                (
                    opcode::Write::new(types::Fd(a.as_raw_fd()), buf.as_ptr(), buf.len() as _)
                        .build(),
                    20,
                )
            });

            assert!(matches!(result, Poll::Pending));

            let t1 = std::thread::spawn(move || {
                let mut buf = [10];
                read(b, &mut buf);
                assert_eq!(buf, [0]);
            });

            let objs = uring.react();

            assert_eq!(objs, vec![20]);

            let result =
                io.submit_or_get_result(|| panic!("Should not be called, as result will be ready"));

            assert!(matches!(result, Poll::Ready(Ok(1))));

            t1.join().unwrap();
        });
    }

    #[test]
    fn multi_events_same_fd_read() {
        run_test(|a, b, uring| {
            let mut buf = [0, 0];

            let mut io1 = uring.new_io();
            assert!(matches!(
                io1.submit_or_get_result(|| {
                    (
                        opcode::Read::new(types::Fd(a.as_raw_fd()), buf.as_mut_ptr(), 1).build(),
                        10,
                    )
                }),
                Poll::Pending
            ));

            let mut io2 = uring.new_io();
            assert!(matches!(
                io2.submit_or_get_result(|| {
                    (
                        opcode::Read::new(types::Fd(a.as_raw_fd()), buf.as_mut_ptr(), 1).build(),
                        20,
                    )
                }),
                Poll::Pending
            ));

            let t1 = std::thread::spawn(move || {
                write(b, &[0xde, 0xad]);
            });

            let objs = uring.react();

            assert_eq!(objs.len(), 2);
            assert!(objs.contains(&10));
            assert!(objs.contains(&20));

            assert!(matches!(
                io1.submit_or_get_result(|| panic!("Should not be called")),
                Poll::Ready(Ok(1))
            ));
            assert!(matches!(
                io2.submit_or_get_result(|| panic!("Should not be called")),
                Poll::Ready(Ok(1))
            ));
            assert_eq!(buf, [0xad, 0]);

            t1.join().unwrap();
        });
    }

    #[test]
    fn multi_events_same_fd_write() {
        run_test(|a, b, uring| {
            let buf = [0xbe, 0xef];

            let mut io1 = uring.new_io();
            assert!(matches!(
                io1.submit_or_get_result(|| {
                    (
                        opcode::Write::new(types::Fd(a.as_raw_fd()), buf.as_ptr(), 2).build(),
                        10,
                    )
                }),
                Poll::Pending
            ));

            let mut io2 = uring.new_io();
            assert!(matches!(
                io2.submit_or_get_result(|| {
                    (
                        opcode::Write::new(types::Fd(a.as_raw_fd()), buf.as_ptr(), 2).build(),
                        20,
                    )
                }),
                Poll::Pending
            ));

            let t1 = std::thread::spawn(move || {
                let mut buf = [0, 0];
                read(b.as_fd(), &mut buf);
                assert_eq!(buf, [0xbe, 0xef]);
                read(b, &mut buf);
            });

            let objs = uring.react();

            assert_eq!(objs.len(), 2);
            assert!(objs.contains(&10));
            assert!(objs.contains(&20));

            assert!(matches!(
                io1.submit_or_get_result(|| panic!("Should not be called")),
                Poll::Ready(Ok(2))
            ));
            assert!(matches!(
                io2.submit_or_get_result(|| panic!("Should not be called")),
                Poll::Ready(Ok(2))
            ));

            t1.join().unwrap();
        });
    }
}
