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

use io_uring::{opcode, types};

use crate::reactor::ReactorIo;

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
    pub(crate) io: ReactorIo,
    pub(crate) buf: &'a mut [u8],
}

impl<T: AsFd + Unpin> Future for AsyncReader<'_, T> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        this.io
            .submit_or_get_result(|| {
                (
                    opcode::Read::new(
                        types::Fd(this.fd.as_fd().as_raw_fd()),
                        this.buf.as_mut_ptr(),
                        this.buf.len() as _,
                    )
                    .build(),
                    cx.waker().clone(),
                )
            })
            .map(|x| x.map(|x| x as _))
    }
}
