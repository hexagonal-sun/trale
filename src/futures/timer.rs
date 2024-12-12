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
//!     Timer::sleep(Duration::from_secs(1)).await;
//!
//!     assert!(now.elapsed() > Duration::from_secs(1));
//!#     Ok::<(), std::io::Error>(())
//! }
//!# );
//! ```

use std::{
    future::Future,
    os::fd::AsFd,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use nix::sys::{
    time::TimeSpec,
    timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
};

use crate::reactor::{Reactor, WakeupKind};

/// Asynchronous timer.
///
/// This structure is a future that will expire at some point in the future. It
/// can be obtained via the [Timer::sleep] function.
pub struct Timer {
    expiration: Instant,
    fd: TimerFd,
}

impl Timer {
    #[must_use]
    /// Put the current task to sleep for the specified duration.
    ///
    /// This function returns a future, that when `.await`ed will suspend the
    /// execution of the current task until the specified duration has elapsed.
    /// At that point the runtime will queue the task for execution. Note that
    /// it is guaranteed that the task will be suspended for *at least* the
    /// specified duration; it could sleep for longer.
    pub fn sleep(d: Duration) -> Self {
        Self {
            expiration: Instant::now() + d,
            fd: TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::empty()).unwrap(),
        }
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if Instant::now() > self.expiration {
            return Poll::Ready(());
        }

        self.fd
            .set(
                Expiration::OneShot(TimeSpec::from(self.expiration - Instant::now())),
                TimerSetTimeFlags::empty(),
            )
            .unwrap();

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
            Timer::sleep(Duration::from_secs(1)).await;
        });
        assert!(Instant::now() - before > Duration::from_millis(900));
    }

    #[test]
    fn sleep_multiple_tasks() {
        let before = Instant::now();
        let t1 = Executor::spawn(async {
            Timer::sleep(Duration::from_secs(1)).await;
        });
        let t2 = Executor::spawn(async {
            Timer::sleep(Duration::from_secs(1)).await;
        });
        let t3 = Executor::spawn(async {
            Timer::sleep(Duration::from_secs(2)).await;
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
            Timer::sleep(Duration::from_secs(1)).await;
            assert!(Instant::now() - before > Duration::from_millis(900));
            assert!(Instant::now() - before < Duration::from_millis(1100));

            let t1 = Executor::spawn(async {
                Timer::sleep(Duration::from_secs(1)).await;
            });
            let t2 = Executor::spawn(async {
                Timer::sleep(Duration::from_secs(1)).await;
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
