use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use std::{
    collections::BTreeMap,
    os::fd::{AsFd, AsRawFd},
    sync::{Arc, Mutex},
};

use super::WakeupKind;

struct RwQueue<T: Send + Sync> {
    readers: Vec<T>,
    writers: Vec<T>,
}

impl<T: Send + Sync> RwQueue<T> {
    fn insert(&mut self, rw: WakeupKind, obj: T) {
        match rw {
            WakeupKind::Readable => self.readers.push(obj),
            WakeupKind::Writable => self.writers.push(obj),
        }
    }

    fn is_empty(&self) -> bool {
        self.readers.is_empty() && self.writers.is_empty()
    }

    fn get(&mut self, flag: EpollFlags) -> Option<T> {
        match flag {
            EpollFlags::EPOLLIN => self.readers.pop(),
            EpollFlags::EPOLLOUT => self.writers.pop(),
            _ => self.readers.pop().or(self.writers.pop()),
        }
    }

    fn new() -> Self {
        Self {
            readers: Vec::new(),
            writers: Vec::new(),
        }
    }
}

pub struct Subscription<T: Send + Sync> {
    fd: Arc<dyn AsFd + Send + Sync>,
    queue: RwQueue<T>,
}

pub struct Poll<T: Send + Sync> {
    subscriptions: Arc<Mutex<BTreeMap<i32, Subscription<T>>>>,
    epoll: Epoll,
}

impl<T: Send + Sync> Poll<T> {
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            epoll: Epoll::new(EpollCreateFlags::empty()).unwrap(),
        }
    }

    pub fn insert(&self, fd: Arc<dyn AsFd + Send + Sync>, data: T, kind: WakeupKind) {
        let mut subscriptions = self.subscriptions.lock().unwrap();
        let idx = fd.as_fd().as_raw_fd();
        if let Some(entry) = subscriptions.get_mut(&idx) {
            entry.queue.insert(kind, data);
        } else {
            let mut sub = Subscription {
                fd: fd.clone(),
                queue: RwQueue::new(),
            };

            sub.queue.insert(kind, data);

            let epoll_event =
                EpollEvent::new(EpollFlags::EPOLLIN | EpollFlags::EPOLLOUT, idx as u64);
            self.epoll.add(fd.as_fd(), epoll_event).unwrap();

            subscriptions.insert(idx, sub);
        }
    }

    pub fn wait(&self) -> T {
        loop {
            let mut event = [EpollEvent::empty()];
            let n = self.epoll.wait(&mut event, EpollTimeout::NONE).unwrap();
            dbg!(event);

            assert_eq!(n, 1);

            {
                let idx = event[0].data() as i32;
                let mut subscriptions = self.subscriptions.lock().unwrap();

                if let Some(sub) = subscriptions.get_mut(&idx) {
                    if let Some(ret) = sub.queue.get(event[0].events()) {
                        if sub.queue.is_empty() {
                            let sub = subscriptions.remove(&idx).unwrap();
                            self.epoll.delete(sub.fd.as_fd()).unwrap();
                        }
                        return ret;
                    }
                }
            }
        }
    }
}
