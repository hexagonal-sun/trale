use std::{
    future::Future,
    io,
    os::fd::{AsFd, AsRawFd},
    pin::Pin,
    task::{Context, Poll},
};

use nix::{errno::Errno, unistd::write};

use crate::reactor::{Reactor, WakeupKind};

pub trait AsyncWrite {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>>;
}

pub struct AsyncWriter<'a, T: AsFd + Unpin> {
    pub(crate) fd: T,
    pub(crate) buf: &'a [u8],
}

impl<T: AsFd + Unpin> Future for AsyncWriter<'_, T> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match write(self.fd.as_fd(), self.buf) {
            Ok(len) => Poll::Ready(Ok(len)),
            Err(e) if e == Errno::EWOULDBLOCK || e == Errno::EAGAIN => {
                Reactor::get().register_waker(
                    self.fd.as_fd().as_raw_fd(),
                    cx.waker().clone(),
                    WakeupKind::Writable,
                );
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }
}
