//! ### Async Mutexes
//!
//! This module provides **cross-thread, non-blocking mutexes** for
//! synchronization in asynchronous contexts. Unlike traditional mutexes, when a
//! `Mutex` is already locked, an attempt to acquire it will **not block** the
//! executor. Instead, the task will yield, allowing other tasks to continue
//! executing. Once the mutex is unlocked, the waiting task will be resumed.
//!
//! The API is designed to be as close to `std::sync::Mutex` as possible, with
//! some differences to support asynchronous operation.
//!
//! #### Example
//!
//! ```rust
//! use trale::task::Executor;
//! use std::sync::Arc;
//! use trale::futures::mutex::Mutex;
//! use trale::futures::timer::Timer;
//! use std::time::Duration;
//! use std::thread;
//!
//! let cell = Arc::new(Mutex::new(0).unwrap());
//!
//! {
//!     let cell = cell.clone();
//!     Executor::spawn(async move {
//!         let mut cell = cell.lock().await;
//!         Timer::sleep(Duration::from_secs(1)).unwrap().await;
//!         *cell += 1;
//!     });
//! }
//!
//! {
//!     let cell = cell.clone();
//!     Executor::spawn(async move {
//!             Timer::sleep(Duration::from_millis(100)).unwrap().await;
//!             *cell.lock().await += 1;
//!     });
//! };
//!
//! Executor::run();
//! Executor::block_on(async move { assert_eq!(*cell.lock().await, 2); });
//! ```
//! In this example:
//!
//! 1. We create a shared Mutex wrapped in an Arc to allow safe concurrent
//!    access across threads.
//! 2. We spawn two asynchronous tasks:
//!     - The first task locks the mutex, then sleeps for 1 second before
//!       modifying the value inside the mutex.
//!     - The second task sleeps for 100 milliseconds, then locks the mutex and
//!       increments the value.
//! 3. By using async mutexes, the tasks can proceed without blocking the
//!    executor, allowing the tasks to run concurrently.
//! 4. After both tasks finish, we assert that the value inside the mutex has
//!    been incremented correctly.
//!
//! Without an async-aware mutex, if one task holds the lock, the other would
//! block the thread, causing a deadlock in the main thread because the async
//! runtime wouldn't be able to progress.
use super::event::Event;
use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
};

/// An async-aware mutex
///
/// See the [module-level documentation](self) for more information.
pub struct Mutex<T> {
    evt: Event,
    obj: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

/// A lock held by a task.
///
/// The `MutexGuard` type represents a **locked** state of a `Mutex`. It is
/// returned when a task successfully locks the mutex via [Mutex::lock], and it
/// automatically releases the lock when it is dropped.
///
/// - The `MutexGuard` ensures that the lock is released as soon as the guard
///   goes out of scope, preventing accidental deadlocks and making sure the mutex
///   is unlocked when no longer needed.
/// - It implements `DerefMut`, so you can mutate the data inside the mutex
///   directly through the guard.
pub struct LockGuard<'a, T> {
    mtx: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    /// Create a new mutex by taking an initial value of an object. If the mutex
    /// fails to be created, then the value is dropped and an error returned. If
    /// the mutex was created, then the mutex which wraps the object is
    /// returned.
    pub fn new(obj: T) -> std::io::Result<Self> {
        let evt = Event::new()?;
        evt.notify_one()?;

        Ok(Self {
            evt,
            obj: UnsafeCell::new(obj),
        })
    }

    /// Attempt to lock the mutex. This function should be `.await`ex to allow
    /// blocking if the mutex is already locked. If the mutex isn't locked, then
    /// the [LockGuard] is returned without yielding. If the mutex is locked,
    /// then the task is put to sleep and will be rescheduled by the run-time
    /// once the mutex has been unlocked by another task.
    pub async fn lock(&self) -> LockGuard<T> {
        let evt = self.evt.clone();
        evt.wait().await.unwrap();
        LockGuard { mtx: self }
    }
}

impl<T> Deref for LockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mtx.obj.get() }
    }
}

impl<T> DerefMut for LockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mtx.obj.get() }
    }
}

impl<T> Drop for LockGuard<'_, T> {
    fn drop(&mut self) {
        self.mtx.evt.notify_one().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::Mutex;
    use crate::{futures::timer::Timer, task::Executor};
    use anyhow::Result;
    use std::{sync::Arc, time::Duration};

    #[test]
    fn simple() -> Result<()> {
        let v = Arc::new(Mutex::new(vec![0u8])?);
        let v2 = v.clone();

        Executor::block_on(async move {
            let mut lock = v.lock().await;

            let v2 = v.clone();

            let t2 = Executor::spawn(async move {
                v2.lock().await.push(2);
            });
            Timer::sleep(Duration::from_millis(250)).unwrap().await;
            lock.push(1);

            drop(lock);

            t2.await;
        });

        assert_eq!(Arc::into_inner(v2).unwrap().obj.into_inner(), vec![0, 1, 2]);

        Ok(())
    }

    #[test]
    fn multiple_locks() -> Result<()> {
        let vv = Arc::new(Mutex::new(vec![0u8])?);
        let v2 = vv.clone();
        Executor::block_on(async move {
            let mut lock = vv.lock().await;
            let v = vv.clone();

            let t2 = Executor::spawn(async move {
                v.lock().await.push(2);
            });
            let v = vv.clone();

            let t3 = Executor::spawn(async move {
                v.lock().await.push(2);
            });
            let v = vv.clone();

            let t4 = Executor::spawn(async move {
                v.lock().await.push(2);
            });
            let v = vv.clone();

            let t5 = Executor::spawn(async move {
                Timer::sleep(Duration::from_millis(500)).unwrap().await;
                v.lock().await.push(2);
            });
            Timer::sleep(Duration::from_millis(500)).unwrap().await;
            lock.push(1);

            drop(lock);

            t2.await;
            t3.await;
            t5.await;
            t4.await;
        });

        assert_eq!(
            Arc::into_inner(v2).unwrap().obj.into_inner(),
            vec![0, 1, 2, 2, 2, 2]
        );

        Ok(())
    }
}
