use std::task::Waker;

use uring::{MultishotUringIo, OneshotUringIo, ReactorUring};

mod uring;

pub type ReactorIo = OneshotUringIo<Waker>;
pub type MultishotReactorIo = MultishotUringIo<Waker>;

pub(crate) struct Reactor {}

thread_local! {
    static REACTOR: ReactorUring<Waker> = ReactorUring::new();
}

impl Reactor {
    pub fn new_io() -> ReactorIo {
        REACTOR.with(|r| r.new_oneshot_io())
    }

    pub fn new_multishot_io() -> MultishotReactorIo {
        REACTOR.with(|r| r.new_multishot_io())
    }

    pub fn react() {
        REACTOR.with(|r| {
            for waker in r.react() {
                waker.wake();
            }
        })
    }
}
