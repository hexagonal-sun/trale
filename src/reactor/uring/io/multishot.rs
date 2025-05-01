use std::{cell::RefCell, rc::Rc, task::Poll};

use io_uring::{squeue, types::CancelBuilder};

use crate::reactor::uring::{result::MultishotResult, IoKind, ReactorInner};

use super::reactor_value_to_result;

#[derive(Debug)]
enum IoState {
    New,
    Submitted(usize, u64),
}

pub(crate) struct MultishotUringIo<T> {
    state: IoState,
    ring: Rc<RefCell<ReactorInner<T>>>,
}

impl<T> MultishotUringIo<T> {
    pub(crate) fn new(ring: Rc<RefCell<ReactorInner<T>>>) -> Self {
        Self {
            state: IoState::New,
            ring,
        }
    }

    pub fn submit_or_get_result(
        &mut self,
        f: impl FnOnce() -> (squeue::Entry, T),
    ) -> Poll<Option<std::io::Result<i32>>> {
        match self.state {
            IoState::New => {
                let (entry, obj) = f();
                let (user_data, result_slot) =
                    self.ring.borrow_mut().submit_io(entry, obj, IoKind::Multi);
                self.state = IoState::Submitted(result_slot, user_data);
                Poll::Pending
            }
            IoState::Submitted(slot, _) => {
                let mut ring = self.ring.borrow_mut();
                let result_store = ring.results.get_multishot();

                match result_store.pop_result(slot) {
                    MultishotResult::Value(v) => Poll::Ready(Some(reactor_value_to_result(v))),
                    MultishotResult::Pending => Poll::Pending,
                    MultishotResult::Finished => Poll::Ready(None),
                }
            }
        }
    }
}

impl<T> Drop for MultishotUringIo<T> {
    fn drop(&mut self) {
        if let IoState::Submitted(slot, user_data) = self.state {
            let mut ring = self.ring.borrow_mut();

            ring.uring
                .submitter()
                .register_sync_cancel(None, CancelBuilder::user_data(user_data))
                .expect("Should be able to cancel in-flight multishot IO");

            ring.results.get_multishot().drop_result(slot);
        }
    }
}
