use super::event::Event;
use anyhow::Result;
use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
};

pub struct Mutex<T> {
    evt: Event,
    obj: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

pub struct LockGuard<'a, T> {
    mtx: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    pub fn new(obj: T) -> Result<Self> {
        let evt = Event::new()?;
        evt.notify_one()?;

        Ok(Self {
            evt,
            obj: UnsafeCell::new(obj),
        })
    }

    pub async fn lock(&self) -> LockGuard<T> {
        let evt = self.evt.clone();
        evt.wait().await.unwrap();
        LockGuard { mtx: &self }
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

        Executor::spawn(async move {
            let mut lock = v.lock().await;

            let v2 = v.clone();

            let t2 = Executor::spawn(async move {
                v2.lock().await.push(2);
            });
            Timer::sleep(Duration::from_millis(250)).await;
            lock.push(1);

            drop(lock);

            t2.await;
        })
        .join();

        assert_eq!(Arc::into_inner(v2).unwrap().obj.into_inner(), vec![0, 1, 2]);

        Ok(())
    }

    #[test]
    fn multiple_locks() -> Result<()> {
        let vv = Arc::new(Mutex::new(vec![0u8])?);
        let v2 = vv.clone();
        Executor::spawn(async move {
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
                Timer::sleep(Duration::from_millis(500)).await;
                v.lock().await.push(2);
            });
            Timer::sleep(Duration::from_millis(500)).await;
            lock.push(1);

            drop(lock);

            t2.await;
            t3.await;
            t5.await;
            t4.await;
        })
        .join();

        assert_eq!(
            Arc::into_inner(v2).unwrap().obj.into_inner(),
            vec![0, 1, 2, 2, 2, 2]
        );

        Ok(())
    }
}
