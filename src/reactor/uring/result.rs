use ringbuffer::{ConstGenericRingBuffer, RingBuffer};
use slab::Slab;

pub(super) enum ResultState {
    Pending,
    Set(i32),
    Dropped,
}

pub(crate) struct OneshotStore(Slab<ResultState>);

impl OneshotStore {
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

    pub fn get_result(&mut self, idx: usize) -> Option<i32> {
        let res = match self.0.get(idx).unwrap() {
            ResultState::Pending => None,
            ResultState::Set(result) => {
                let ret = Some(*result);
                self.0.remove(idx);
                ret
            }
            ResultState::Dropped => panic!("Should not be able to get a dropped result"),
        };

        res
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

struct MultishotResultState {
    results: ConstGenericRingBuffer<i32, 1024>,
    dropped: bool,
    finished: bool,
}

pub enum MultishotResult {
    Value(i32),
    Pending,
    Finished,
}

pub(crate) struct MultishotStore(Slab<MultishotResultState>);

impl MultishotStore {
    fn new() -> Self {
        Self(Slab::new())
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push_result(&mut self, result: i32, idx: usize) {
        self.0.get_mut(idx).unwrap().results.push(result);
    }

    pub fn pop_result(&mut self, idx: usize) -> MultishotResult {
        let result = self.0.get_mut(idx).unwrap();

        match result.results.dequeue() {
            Some(v) => MultishotResult::Value(v),
            None => {
                if result.finished {
                    MultishotResult::Finished
                } else {
                    MultishotResult::Pending
                }
            }
        }
    }

    pub fn drop_result(&mut self, idx: usize) {
        if self.0.get_mut(idx).unwrap().finished {
            self.0.remove(idx);
        } else {
            self.0.get_mut(idx).unwrap().dropped = true;
        }
    }

    pub fn create_slot(&mut self) -> usize {
        self.0.insert(MultishotResultState {
            results: ConstGenericRingBuffer::new(),
            dropped: false,
            finished: false,
        })
    }

    pub fn set_finished(&mut self, idx: usize) {
        if self.0.get(idx).unwrap().dropped {
            self.0.remove(idx);
        } else {
            self.0.get_mut(idx).unwrap().finished = true;
        }
    }
}

pub struct RingResults {
    oneshot: OneshotStore,
    multishot: MultishotStore,
}

impl RingResults {
    pub fn new() -> Self {
        Self {
            oneshot: OneshotStore::new(),
            multishot: MultishotStore::new(),
        }
    }

    pub fn get_oneshot(&mut self) -> &mut OneshotStore {
        &mut self.oneshot
    }

    pub fn get_multishot(&mut self) -> &mut MultishotStore {
        &mut self.multishot
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.oneshot.is_empty() && self.multishot.is_empty()
    }
}
