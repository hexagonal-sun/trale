use std::{cell::RefCell, task::Poll};

use super::{IoKind, ReactorInner};
use io_uring::squeue;

#[derive(Debug)]
enum IoState {
    New,
    Submitted(usize),
    Finished(i32),
}

pub(crate) struct UringIo<'a, T> {
    state: IoState,
    kind: IoKind,
    ring: &'a RefCell<ReactorInner<T>>,
}

impl From<&IoState> for Poll<std::io::Result<i32>> {
    fn from(value: &IoState) -> Self {
        match value {
            IoState::New => Poll::Pending,
            IoState::Submitted(_) => Poll::Pending,
            IoState::Finished(result) => Poll::Ready(if *result < 0 {
                Err(std::io::Error::from_raw_os_error(result.abs()))
            } else {
                Ok(*result)
            }),
        }
    }
}

impl<'a, T> UringIo<'a, T> {
    pub(super) fn new(ring: &'a RefCell<ReactorInner<T>>, kind: IoKind) -> Self {
        Self {
            state: IoState::New,
            kind,
            ring,
        }
    }

    pub fn submit_or_get_result(
        &mut self,
        f: impl FnOnce() -> (squeue::Entry, T),
    ) -> Poll<std::io::Result<i32>> {
        match self.state {
            IoState::New => {
                let (entry, obj) = f();
                let result_slot = self.ring.borrow_mut().submit_io(entry, obj, self.kind);
                self.state = IoState::Submitted(result_slot);
            }
            IoState::Submitted(slot) => {
                let mut ring = self.ring.borrow_mut();
                let result_store = ring.results.get(self.kind);

                if let Some(res) = result_store.pop_result(slot) {
                    match self.kind {
                        IoKind::Oneshot => self.state = IoState::Finished(res),
                        IoKind::Multi => return (&IoState::Finished(res)).into(),
                    };
                }
            }
            IoState::Finished(_) => {}
        }

        (&self.state).into()
    }
}

impl<'a, T> Drop for UringIo<'a, T> {
    fn drop(&mut self) {
        if let IoState::Submitted(slot) = self.state {
            self.ring
                .borrow_mut()
                .results
                .get(self.kind)
                .drop_result(slot);
        }
    }
}
