use std::{
    future::Future,
    io,
    os::fd::{AsFd, AsRawFd},
    pin::Pin,
    task::{Context, Poll},
};

use libc::EWOULDBLOCK;

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
        let res = unsafe {
            libc::read(
                self.fd.as_fd().as_raw_fd(),
                self.buf.as_mut_ptr() as *mut _,
                self.buf.len() as _,
            )
        };

        if res != -1 {
            return Poll::Ready(Ok(res as usize));
        }

        let err = std::io::Error::last_os_error();

        match err.raw_os_error().unwrap() {
            EWOULDBLOCK => {
                Reactor::get().register_waker(
                    self.fd.as_fd().as_raw_fd(),
                    cx.waker().clone(),
                    WakeupKind::Writable,
                );
                Poll::Pending
            }
            _ => Poll::Ready(Err(err)),
        }
    }
}
