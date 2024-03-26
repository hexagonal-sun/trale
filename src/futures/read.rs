use std::{
    future::Future,
    io,
    os::fd::{AsRawFd, OwnedFd},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use nix::{errno::Errno, unistd::read};

use crate::reactor::{Reactor, WakeupKind};

pub trait AsyncRead {
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>>;
}

pub struct AsyncReader<'a> {
    pub(crate) fd: Arc<OwnedFd>,
    pub(crate) buf: &'a mut [u8],
}

impl Future for AsyncReader<'_> {
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut();
        match read(this.fd.as_raw_fd(), this.buf) {
            Ok(len) => Poll::Ready(Ok(len)),
            Err(e) if e == Errno::EWOULDBLOCK => {
                Reactor::get().register_waker(
                    self.fd.clone(),
                    cx.waker().clone(),
                    WakeupKind::Readable,
                );
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }
}
