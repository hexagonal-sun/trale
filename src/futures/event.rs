//! Async event synchronisation.
//!
//! This module implements an event synchroniser between tasks. It is backed by
//! the Linux kernel's
//! [eventfd](https://man7.org/linux/man-pages/man2/eventfd.2.html). It allows
//! one task to inform another that an event has taken place.
//!
//! # Example
//!
//! Here is an example of one task that waits for an event from another.
//! ```
//! use trale::futures::event::Event;
//! use trale::futures::timer::Timer;
//! use trale::task::Executor;
//! use std::time::Duration;
//!# Executor::block_on(
//! async {
//!     let evt = Event::new()?;
//!     let evt2 = evt.clone();
//!     let tsk = Executor::spawn(async move {
//!         evt2.wait().await.unwrap();
//!     });
//!
//!     Timer::sleep(Duration::from_secs(1)).unwrap().await;
//!
//!     evt.notify_one()?;
//!
//!     tsk.await;
//!#     Ok::<(), std::io::Error>(())
//! }
//!# );
//! ```
//!
//! Note that the events can also act as a semaphore, allowing multiple events
//! to be queued up and awaited:
//! ```
//! use trale::futures::event::Event;
//! use trale::futures::timer::Timer;
//! use trale::task::Executor;
//! use std::time::Duration;
//! use std::thread;
//!# Executor::block_on(
//! async {
//!     let evt = Event::new()?;
//!     let evt2 = evt.clone();
//!     let evt3 = evt.clone();
//!     let tsk = Executor::spawn(async move {
//!         let mut count = 0;
//!
//!         while count < 20 {
//!             evt2.wait().await.unwrap();
//!             count += 1;
//!         }
//!     });
//!
//!     thread::spawn(move || {
//!         for _ in 0..10 {
//!             evt3.notify_one();
//!         }
//!     });
//!
//!     for _ in 0..10 {
//!         evt.notify_one();
//!     }
//!
//!     tsk.await;
//!#     Ok::<(), std::io::Error>(())
//! }
//!# );
//! ```
use libc::{eventfd, EFD_NONBLOCK, EFD_SEMAPHORE};
use std::{
    ffi::c_void,
    future::Future,
    io::{ErrorKind, Result},
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
    pin::Pin,
    task::{Context, Poll},
};

use crate::reactor::{Reactor, WakeupKind};

/// Async event signaller
///
/// Allows events to be signaled between async tasks. Use [Event::new] to create an
/// event object, await the [Event::wait] future to suspend execution until an event
/// arrives and [Event::notify_one] to send an event.
#[derive(Debug)]
pub struct Event {
    inner: OwnedFd,
}

/// Event wait future
///
/// A future that will wait for the assoicated Event to become ready. See
/// [Event::wait()].
pub struct EventWaiter<'fd> {
    inner: BorrowedFd<'fd>,
}

impl Event {
    /// Construct a new event object.
    ///
    /// This function calls the `eventfd()` function and returns the wrapped
    /// file descriptor in an Event object. If the `eventfd()` function fails
    /// this function will return `Err` with an associated error object.
    pub fn new() -> Result<Self> {
        let fd = unsafe { eventfd(0, EFD_NONBLOCK | EFD_SEMAPHORE) };

        if fd == -1 {
            Err(std::io::Error::last_os_error())?;
        }

        Ok(Self {
            inner: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    /// Notify of an event.
    ///
    /// This function sets the event as triggered. Any associated waiters will
    /// be awoken and the [EventWaiter] future will complete. If there are no
    /// tasks awaiting the event when this function is called, the event will be
    /// latched such that any subsequent awaits on [Event::wait] will *not*
    /// suspend execution, but will poll as `Ready`.
    pub fn notify_one(&self) -> Result<()> {
        let buffer = 1_u64.to_ne_bytes();
        let ret = unsafe {
            libc::write(
                self.inner.as_raw_fd(),
                buffer.as_ptr() as *const c_void,
                buffer.len(),
            )
        };

        if ret == -1 {
            Err(std::io::Error::last_os_error())?
        }

        if ret as usize != buffer.len() {
            return Err(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                "Failed to write entire event fd buffer",
            ));
        }

        Ok(())
    }

    /// Wait for an event
    ///
    /// This function returns an [EventWaiter] object which can be `.await`ed to
    /// suspend execution until an event is sent via [Event::notify_one]. Note
    /// that if an event has already been sent *before* this function was
    /// called, `.await`ing on the return from this function will return
    /// `Ready`.
    pub fn wait(&self) -> EventWaiter {
        EventWaiter {
            inner: self.inner.as_fd(),
        }
    }
}

impl Clone for Event {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.try_clone().unwrap(),
        }
    }
}

impl Future for EventWaiter<'_> {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut buf = [0_u8; std::mem::size_of::<u64>()];

        let ret = unsafe {
            libc::read(
                self.inner.as_raw_fd(),
                buf.as_mut_ptr() as *mut _,
                buf.len(),
            )
        };

        match if ret == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(ret)
        } {
            Ok(_) => {
                assert_eq!(u64::from_ne_bytes(buf), 1);
                Poll::Ready(Ok(()))
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                Reactor::get().register_waker(
                    self.inner.as_raw_fd(),
                    ctx.waker().clone(),
                    WakeupKind::Readable,
                );
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::Event;
    use crate::task::Executor;

    #[test]
    fn simple() {
        Executor::block_on(async {
            let evt = Event::new().unwrap();

            let task = {
                let evt = evt.clone();
                Executor::spawn(async move {
                    evt.wait().await.unwrap();
                })
            };

            evt.notify_one().unwrap();
            task.await;
        });
    }

    #[test]
    fn multi_notifiers() {
        Executor::block_on(async {
            let evt = Event::new().unwrap();

            let task = {
                let evt = evt.clone();
                Executor::spawn(async move {
                    let mut count = 0;
                    while count < 40 {
                        evt.wait().await.unwrap();
                        count += 1;
                    }
                })
            };

            let t1 = {
                let evt = evt.clone();
                thread::spawn(move || {
                    for _ in 0..20 {
                        evt.notify_one().unwrap();
                    }
                })
            };

            let t2 = {
                let evt = evt.clone();
                thread::spawn(move || {
                    for _ in 0..20 {
                        evt.notify_one().unwrap();
                    }
                })
            };

            t1.join().unwrap();
            t2.join().unwrap();
            task.await;
        });
    }
}
