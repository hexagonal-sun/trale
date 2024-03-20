use std::{
    future::Future, pin::Pin, sync::Arc, task::{Context, Poll}, time::{Duration, Instant}
};

use nix::sys::{
    time::TimeSpec,
    timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
};

use crate::reactor::Reactor;

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

        Reactor::get().register_waker(self.fd.clone(), cx.waker().clone());

        Poll::Pending
    }
}
