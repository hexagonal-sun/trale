use std::{
    future::Future,
    io,
    os::fd::{AsFd, AsRawFd},
    pin::Pin,
    task::{Context, Poll},
};

use nix::{errno::Errno, unistd::read};

use crate::reactor::{Reactor, WakeupKind};

pub trait AsyncRead {
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>>;
}

pub struct AsyncReader<'a, T: AsFd + Unpin> {
    pub(crate) fd: T,
    pub(crate) buf: &'a mut [u8],
}

impl<T: AsFd + Unpin> Future for AsyncReader<'_, T> {
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut();
        match read(this.fd.as_fd().as_raw_fd(), this.buf) {
            Ok(len) => Poll::Ready(Ok(len)),
            Err(e) if e == Errno::EWOULDBLOCK => {
                Reactor::get().register_waker(
                    self.fd.as_fd().as_raw_fd(),
                    cx.waker().clone(),
                    WakeupKind::Readable,
                );
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }
}
