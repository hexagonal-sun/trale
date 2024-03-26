use std::{
    future::Future,
    io,
    os::fd::{AsFd, OwnedFd},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use nix::{errno::Errno, unistd::write};

use crate::reactor::{Reactor, WakeupKind};

pub trait AsyncWrite {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>>;
}

pub struct AsyncWriter<'a> {
    pub(crate) fd: Arc<OwnedFd>,
    pub(crate) buf: &'a [u8],
}

impl Future for AsyncWriter<'_> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match write(self.fd.as_fd(), self.buf) {
            Ok(len) => Poll::Ready(Ok(len)),
            Err(e) if e == Errno::EWOULDBLOCK || e == Errno::EAGAIN => {
                Reactor::get().register_waker(
                    self.fd.clone(),
                    cx.waker().clone(),
                    WakeupKind::Writable,
                );
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }
}
