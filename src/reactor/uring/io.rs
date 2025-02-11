use std::{cell::RefCell, task::Poll};

use io_uring::squeue;
use slab::Slab;

use super::ReactorInner;

#[derive(Debug)]
enum IoState {
    New,
    Submitted(usize),
    Finished(i32),
}

pub(crate) struct UringIo<'a, T> {
    state: IoState,
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
    pub(super) fn new(ring: &'a RefCell<ReactorInner<T>>) -> Self {
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
                let result_slot = self.ring.borrow_mut().submit_io(entry, obj);
                self.state = IoState::Submitted(result_slot);
            }
            IoState::Submitted(slot) => {
                let mut ring = self.ring.borrow_mut();
                if let Some(res) = ring.results.get_result(slot) {
                    self.state = IoState::Finished(res);
                    ring.results.drop_result(slot);
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
            self.ring.borrow_mut().results.drop_result(slot);
        }
    }
}

pub struct RingResults(Slab<ResultState>);

pub(super) enum ResultState {
    Pending,
    Set(i32),
    Dropped,
}

impl RingResults {
    pub fn new() -> Self {
        Self(Slab::new())
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn set_result(&mut self, result: i32, idx: usize) {
        let r_entry = self.0.get_mut(idx).unwrap();

        if matches!(r_entry, ResultState::Dropped) {
            self.0.remove(idx);
        } else {
            *r_entry = ResultState::Set(result);
        }
    }

    pub fn get_result(&self, idx: usize) -> Option<i32> {
        match self.0.get(idx).unwrap() {
            ResultState::Pending => None,
            ResultState::Set(result) => Some(*result),
            ResultState::Dropped => panic!("Should not be able to get a dropped result"),
        }
    }

    pub fn drop_result(&mut self, idx: usize) {
        let r_entry = self.0.get_mut(idx).unwrap();

        if matches!(r_entry, ResultState::Set(_)) {
            self.0.remove(idx);
        } else {
            *r_entry = ResultState::Dropped;
        }
    }

    pub fn create_slot(&mut self) -> usize {
        self.0.insert(ResultState::Pending)
    }
}
