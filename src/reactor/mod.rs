use std::{cell::RefCell, os::fd::AsRawFd, task::Waker};

use self::poll::Poll;

mod poll;

#[derive(Debug)]
pub enum WakeupKind {
    Readable,
    Writable,
}

pub(crate) struct Reactor {
    poll: Poll<Waker>,
}

thread_local! {
    static REACTOR: RefCell<Reactor> = RefCell::new( Reactor {
        poll: Poll::new().unwrap()
    });
}

impl Reactor {
    pub fn register_waker(fd: impl AsRawFd, waker: Waker, kind: WakeupKind) {
        REACTOR.with(|r| {
            r.borrow_mut().poll.insert(fd.as_raw_fd(), waker, kind);
        })
    }

    pub fn react() {
        REACTOR.with(|r| {
            let waker = r.borrow_mut().poll.wait();

            waker.wake();
        })
    }
}
