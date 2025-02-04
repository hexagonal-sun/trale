pub(crate) use io::UringIo;
use std::{mem::transmute, task::Waker};
use uring::ReactorUring;

mod io;
mod uring;

pub type ReactorIo = UringIo<'static, Waker>;

pub(crate) struct Reactor {}

thread_local! {
    static REACTOR: ReactorUring<Waker> = ReactorUring::new();
}

impl Reactor {
    pub fn new_io() -> ReactorIo {
        REACTOR.with(|r| unsafe { transmute(r.new_io()) })
    }

    pub fn react() {
        REACTOR.with(|r| {
            for waker in r.react().into_iter() {
                waker.wake();
            }
        })
    }
}
