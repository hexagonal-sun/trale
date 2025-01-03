//! Asynchronous reads.
//!
//! The `read` module provides functionality for performing asynchronous reads
//! to non-blocking file descriptors (FDs). It allows tasks to attempt reading
//! data from an FD without blocking the executor, enabling efficient handling
//! of I/O operations in an async environment.
//!
//! Various futures within trale implement the `AsyncRead` trait, and those that
//! do will allow you to await a call to [AsyncRead::read].
use std::{
    future::Future,
    io,
    os::fd::{AsFd, AsRawFd},
    pin::Pin,
    task::{Context, Poll},
};

use libc::EWOULDBLOCK;

use crate::reactor::{Reactor, WakeupKind};

/// Asynchronous reads.
///
/// All futures in trale that can be read from will implement this type. You can
/// call the [AsyncRead::read] function to obtain a future which will complete
/// once the requested read has finished (successfully or not).
pub trait AsyncRead {
    /// Return a future that, when `.await`ed will block until the read has been
    /// successful or not. When successful, the number of bytes that were read
    /// is returned. Note that this might be *less* than the number of bytes in
    /// `buf`.
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>>;
}

pub(crate) struct AsyncReader<'a, T: AsFd + Unpin> {
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
                Reactor::register_waker(
                    self.fd.as_fd().as_raw_fd(),
                    cx.waker().clone(),
                    WakeupKind::Readable,
                );
                Poll::Pending
            }
            _ => Poll::Ready(Err(err)),
        }
    }
}
