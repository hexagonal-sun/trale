use std::{
    os::fd::AsFd,
    sync::{Arc, Mutex},
};

use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use slab::Slab;

pub struct Subscription<T: Send + Sync> {
    fd: Arc<dyn AsFd + Send + Sync>,
    data: T,
}

pub struct Poll<T: Send + Sync> {
    wakers: Arc<Mutex<Slab<Subscription<T>>>>,
    epoll: Epoll,
}

impl<T: Send + Sync> Poll<T> {
    pub fn new() -> Self {
        Self {
            wakers: Arc::new(Mutex::new(Slab::new())),
            epoll: Epoll::new(EpollCreateFlags::empty()).unwrap(),
        }
    }

    pub fn insert(&self, fd: Arc<dyn AsFd + Send + Sync>, data: T) {
        let mut slab = self.wakers.lock().unwrap();
        let entry = slab.vacant_entry();
        let sub = Subscription {
            fd: fd.clone(),
            data,
        };

        let mut flags = EpollFlags::EPOLLIN;
        flags.set(EpollFlags::EPOLLET, true);

        let epoll_event = EpollEvent::new(flags, entry.key() as u64,);
        self.epoll.add(fd.as_fd(), epoll_event).unwrap();

        entry.insert(sub);
    }

    pub fn wait(&self) -> T {
        let mut event = [EpollEvent::empty()];
        let n = self.epoll.wait(&mut event, EpollTimeout::NONE).unwrap();

        assert_eq!(n, 1);

        {
            let mut slab = self.wakers.lock().unwrap();
            let subscription = slab.remove(event[0].data() as usize);
            self.epoll
                .delete(subscription.fd.as_fd())
                .unwrap();

            subscription.data
        }
    }
}
