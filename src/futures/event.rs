use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::Result;
use nix::{
    errno::Errno,
    sys::eventfd::{EfdFlags, EventFd},
};

use crate::reactor::{Reactor, WakeupKind};

#[derive(Clone)]
pub struct Event {
    inner: Arc<EventFd>,
}

impl Event {
    pub fn new() -> Result<Self> {
        let inner = EventFd::from_flags(EfdFlags::EFD_NONBLOCK | EfdFlags::EFD_SEMAPHORE)?;

        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    pub fn notify_one(&self) -> Result<()> {
        self.inner.arm()?;

        Ok(())
    }
}

impl Future for Event {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.read() {
            Ok(_) => Poll::Ready(Ok(())),
            Err(Errno::EWOULDBLOCK) => {
                Reactor::get().register_waker(
                    self.inner.clone(),
                    ctx.waker().clone(),
                    WakeupKind::Readable,
                );
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }
}
