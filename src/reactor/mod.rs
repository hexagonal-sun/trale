use async_lock::OnceCell;
use std::{os::fd::AsRawFd, task::Waker, thread};

use self::poll::Poll;

mod poll;

pub struct Reactor {
    poll: Poll<Waker>,
}

pub enum WakeupKind {
    Readable,
    Writable,
}

impl Reactor {
    pub fn get() -> &'static Self {
        static REACTOR: OnceCell<Reactor> = OnceCell::new();

        REACTOR.get_or_init_blocking(|| {
            thread::spawn(|| Self::reactor_loop());

            Self {
                poll: Poll::new().unwrap(),
            }
        })
    }

    pub fn register_waker(&self, fd: impl AsRawFd, waker: Waker, kind: WakeupKind) {
        self.poll.insert(fd.as_raw_fd(), waker, kind);
    }

    fn reactor_loop() -> ! {
        let reactor = Self::get();

        loop {
            let waker = reactor.poll.wait();

            waker.wake();
        }
    }
}
