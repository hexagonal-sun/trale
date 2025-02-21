use ringbuffer::{ConstGenericRingBuffer, RingBuffer};
use slab::Slab;

use super::IoKind;

pub trait ResultStore {
    fn set_result(&mut self, result: i32, idx: usize);
    fn pop_result(&mut self, idx: usize) -> Option<i32>;
    fn drop_result(&mut self, idx: usize);
    fn create_slot(&mut self) -> usize;
}

pub(super) enum ResultState {
    Pending,
    Set(i32),
    Dropped,
}

struct OneshotStore(Slab<ResultState>);

impl OneshotStore {
    pub fn new() -> Self {
        Self(Slab::new())
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl ResultStore for OneshotStore {
    fn set_result(&mut self, result: i32, idx: usize) {
        let r_entry = self.0.get_mut(idx).unwrap();

        if matches!(r_entry, ResultState::Dropped) {
            self.0.remove(idx);
        } else {
            *r_entry = ResultState::Set(result);
        }
    }

    fn pop_result(&mut self, idx: usize) -> Option<i32> {
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

    fn drop_result(&mut self, idx: usize) {
        let r_entry = self.0.get_mut(idx).unwrap();

        if matches!(r_entry, ResultState::Set(_)) {
            self.0.remove(idx);
        } else {
            *r_entry = ResultState::Dropped;
        }
    }

    fn create_slot(&mut self) -> usize {
        self.0.insert(ResultState::Pending)
    }
}

enum MultishotResultState {
    Active(ConstGenericRingBuffer<i32, 1024>),
    Dropped,
}

struct MultishotStore(Slab<MultishotResultState>);

impl MultishotStore {
    fn new() -> Self {
        Self(Slab::new())
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl ResultStore for MultishotStore {
    fn set_result(&mut self, result: i32, idx: usize) {
        let r_entry = self.0.get_mut(idx).unwrap();

        match r_entry {
            MultishotResultState::Active(ref mut ring) => {
                ring.push(result);
            }

            // If the IO has been dropped, ignore any results.
            MultishotResultState::Dropped => {}
        }
    }

    fn pop_result(&mut self, idx: usize) -> Option<i32> {
        match self.0.get_mut(idx).unwrap() {
            MultishotResultState::Active(ref mut ring) => ring.dequeue(),
            MultishotResultState::Dropped => panic!("Shoult not be able to get a dropped result"),
        }
    }

    fn drop_result(&mut self, idx: usize) {
        let r_entry = self.0.get_mut(idx).unwrap();
        *r_entry = MultishotResultState::Dropped;
    }

    fn create_slot(&mut self) -> usize {
        self.0
            .insert(MultishotResultState::Active(ConstGenericRingBuffer::new()))
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

    pub fn get(&mut self, kind: IoKind) -> &mut dyn ResultStore {
        match kind {
            IoKind::Oneshot => &mut self.oneshot,
            IoKind::Multi => &mut self.multishot,
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.oneshot.is_empty() && self.multishot.is_empty()
    }
}
