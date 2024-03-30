use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use nix::sys::{
    time::TimeSpec,
    timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
};

use crate::reactor::{Reactor, WakeupKind};

pub struct Timer {
    expiration: Instant,
    fd: Arc<TimerFd>,
}

impl Timer {
    #[must_use]
    pub fn sleep(d: Duration) -> Self {
        Self {
            expiration: Instant::now() + d,
            fd: Arc::new(TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::empty()).unwrap()),
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

        Reactor::get().register_waker(self.fd.clone(), cx.waker().clone(), WakeupKind::Readable);

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
