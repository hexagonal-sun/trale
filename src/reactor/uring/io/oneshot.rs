use std::{cell::RefCell, rc::Rc, task::Poll};

use io_uring::squeue;

use crate::reactor::uring::{IoKind, ReactorInner};

use super::reactor_value_to_result;

#[derive(Debug)]
enum IoState {
    New,
    Submitted(usize),
    Finished(i32),
}

pub(crate) struct OneshotUringIo<T> {
    state: IoState,
    ring: Rc<RefCell<ReactorInner<T>>>,
}

impl From<&IoState> for Poll<std::io::Result<i32>> {
    fn from(value: &IoState) -> Self {
        match value {
            IoState::New => Poll::Pending,
            IoState::Submitted(_) => Poll::Pending,
            IoState::Finished(result) => Poll::Ready(reactor_value_to_result(*result)),
        }
    }
}

impl<T> OneshotUringIo<T> {
    pub fn new(ring: Rc<RefCell<ReactorInner<T>>>) -> Self {
        Self {
            state: IoState::New,
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
                let (_, result_slot) =
                    self.ring
                        .borrow_mut()
                        .submit_io(entry, obj, IoKind::Oneshot);
                self.state = IoState::Submitted(result_slot);
            }
            IoState::Submitted(slot) => {
                let mut ring = self.ring.borrow_mut();
                let result_store = ring.results.get_oneshot();

                if let Some(res) = result_store.get_result(slot) {
                    self.state = IoState::Finished(res);
                }
            }
            IoState::Finished(_) => {}
        }

        (&self.state).into()
    }
}

impl<T> Drop for OneshotUringIo<T> {
    fn drop(&mut self) {
        if let IoState::Submitted(slot) = self.state {
            let mut ring = self.ring.borrow_mut();

            ring.results.get_oneshot().drop_result(slot);
        }
    }
}
