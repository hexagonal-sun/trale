//! Asynchronous writes.
//!
//! The `write` module provides functionality for performing asynchronous writes
//! to non-blocking file descriptors (FDs). It allows tasks to attempt writing
//! data to an FD without blocking the executor, enabling efficient handling of
//! I/O operations in an async environment.
//!
//! Various futures within trale implement the `AsyncWrite` trait, and those
//! that do will allow you to await a call to [AsyncWrite::write].
use std::{
    future::Future,
    io,
    os::fd::{AsFd, AsRawFd},
    pin::Pin,
    task::{Context, Poll},
};

use libc::EWOULDBLOCK;

use crate::reactor::{Reactor, WakeupKind};

/// Asynchronous writes.
///
/// All futures in trale that can be written to will implement this type. You
/// can call the [AsyncWrite::write] function to obtain a future which will
/// complete once the requested write has finished (successfully or not).
pub trait AsyncWrite {
    /// Return a future that, when `.await`ed will block until the write has
    /// been successful or not. When successful, the number of bytes that were
    /// written is returned. Note that this might be *less* than the number of
    /// bytes in `buf`.
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>>;
}

pub(crate) struct AsyncWriter<'a, T: AsFd + Unpin> {
    pub(crate) fd: T,
    pub(crate) buf: &'a [u8],
}

impl<T: AsFd + Unpin> Future for AsyncWriter<'_, T> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = unsafe {
            libc::write(
                self.fd.as_fd().as_raw_fd(),
                self.buf.as_ptr() as *mut _,
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
                    WakeupKind::Writable,
                );
                Poll::Pending
            }
            _ => Poll::Ready(Err(err)),
        }
    }
}
