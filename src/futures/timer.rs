//! Async timer related futures.
//!
//! This module uses the Linux kernel's
//! [timerfd](https://man7.org/linux/man-pages/man2/timerfd_create.2.html)
//! facility to implement asynchronous timers. The main use-case for this is to
//! put a task to sleep for a specific period of time.
//!
//! # Example
//! Let's put a task to sleep for 2 seconds.
//! ```
//! use trale::futures::timer::Timer;
//! use trale::task::Executor;
//! use std::time::{Duration, Instant};
//!# Executor::block_on(
//! async {
//!     let now = Instant::now();
//!
//!     Timer::sleep(Duration::from_secs(2)).unwrap().await;
//!
//!     assert!(now.elapsed() > Duration::from_secs(2));
//!#     Ok::<(), std::io::Error>(())
//! }
//!# );
//! ```

use std::{
    future::Future,
    io::Result,
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    pin::Pin,
    ptr::null_mut,
    task::{Context, Poll},
    time::{Duration, SystemTime},
};

use libc::{CLOCK_MONOTONIC, TFD_NONBLOCK};
use crate::reactor::{Reactor, WakeupKind};

/// Asynchronous timer.
///
/// This structure is a future that will expire at some point in the future. It
/// can be obtained via the [Timer::sleep] function.
pub struct Timer {
    expiration: SystemTime,
    fd: OwnedFd,
}

impl Timer {
    fn compute_tspec(&self) -> libc::itimerspec {
        let expiration = self.expiration.duration_since(SystemTime::now()).unwrap();
        let mut tspec = unsafe { std::mem::zeroed::<libc::itimerspec>() };

        tspec.it_value.tv_sec = expiration.as_secs() as _;
        tspec.it_value.tv_nsec = expiration.subsec_nanos() as _;

        tspec
    }

    #[must_use]
    /// Put the current task to sleep for the specified duration.
    ///
    /// This function returns a future, that when `.await`ed will suspend the
    /// execution of the current task until the specified duration has elapsed.
    /// At that point the runtime will queue the task for execution. Note that
    /// it is guaranteed that the task will be suspended for *at least* the
    /// specified duration; it could sleep for longer.
    pub fn sleep(d: Duration) -> Result<Self> {
        let expiration = SystemTime::now() + d;
        let timer = unsafe { libc::timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK) };

        if timer == -1 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            expiration,
            fd: unsafe { OwnedFd::from_raw_fd(timer) },
        })
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if SystemTime::now() > self.expiration {
            return Poll::Ready(());
        }

        let tspec = self.compute_tspec();

        let ret = unsafe {
            libc::timerfd_settime(self.fd.as_raw_fd(), 0, &tspec as *const _, null_mut())
        };

        if ret == -1 {
            panic!("timerfd_settime returned error");
        }

        Reactor::get().register_waker(self.fd.as_fd(), cx.waker().clone(), WakeupKind::Readable);

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::task::Executor;

    use super::Timer;

    #[test]
    fn sleep_simple() {
        let before = Instant::now();
        Executor::block_on(async {
            Timer::sleep(Duration::from_secs(1)).unwrap().await;
        });
        assert!(Instant::now() - before > Duration::from_millis(900));
    }

    #[test]
    fn sleep_multiple_tasks() {
        let before = Instant::now();
        let t1 = Executor::spawn(async {
            Timer::sleep(Duration::from_secs(1)).unwrap().await;
        });
        let t2 = Executor::spawn(async {
            Timer::sleep(Duration::from_secs(1)).unwrap().await;
        });
        let t3 = Executor::spawn(async {
            Timer::sleep(Duration::from_secs(2)).unwrap().await;
        });

        t1.join();
        t2.join();
        assert!(Instant::now() - before > Duration::from_millis(900));
        assert!(Instant::now() - before < Duration::from_millis(1100));

        t3.join();
        assert!(Instant::now() - before > Duration::from_millis(1900));
        assert!(Instant::now() - before < Duration::from_millis(2100));
    }

    #[test]
    fn sleep_subtasks() {
        let before = Instant::now();
        Executor::block_on(async move {
            Timer::sleep(Duration::from_secs(1)).unwrap().await;
            assert!(Instant::now() - before > Duration::from_millis(900));
            assert!(Instant::now() - before < Duration::from_millis(1100));

            let t1 = Executor::spawn(async {
                Timer::sleep(Duration::from_secs(1)).unwrap().await;
            });
            let t2 = Executor::spawn(async {
                Timer::sleep(Duration::from_secs(1)).unwrap().await;
            });

            t1.await;
            t2.await;
            assert!(Instant::now() - before > Duration::from_millis(1900));
            assert!(Instant::now() - before < Duration::from_millis(2100));
        });
        assert!(Instant::now() - before > Duration::from_millis(1900));
        assert!(Instant::now() - before < Duration::from_millis(2100));
    }
}
