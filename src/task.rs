use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{sync_channel, Receiver},
        Arc, Condvar, Mutex,
    },
    task::{Context, Poll, Wake, Waker},
    thread,
};

use async_lock::OnceCell;
use slab::Slab;

struct Task {
    id: AtomicUsize,
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send + Sync>>>,
}

impl Wake for Task {
    fn wake(self: std::sync::Arc<Self>) {
        let executor = Executor::get();

        {
            let mut waiting = executor.waiting.lock().unwrap();
            waiting.remove(self.id.load(Ordering::Relaxed));
        }

        {
            let mut run_q = executor.run_q.lock().unwrap();
            run_q.push(self);
            Executor::get().cv.notify_all();
        }
    }
}

pub struct Executor {
    waiting: Mutex<Slab<Arc<Task>>>,
    run_q: Mutex<Vec<Arc<Task>>>,
    cv: Condvar,
}

pub struct TaskJoiner<T> {
    rx: Receiver<T>,
}

impl<T> TaskJoiner<T> {
    pub fn join(self) -> T {
        self.rx.recv().unwrap()
    }
}

impl Executor {
    fn get() -> &'static Self {
        static EXECUTOR: OnceCell<Executor> = OnceCell::new();

        EXECUTOR.get_or_init_blocking(|| {
            thread::spawn(Self::executor_loop);

            Self {
                waiting: Mutex::new(Slab::new()),
                run_q: Mutex::new(Vec::new()),
                cv: Condvar::new(),
            }
        })
    }

    pub fn spawn<Fut, T>(f: Fut) -> TaskJoiner<T>
    where
        Fut: Future<Output = T> + Sync + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = sync_channel(1);

        let fut = async move {
            let value = f.await;
            tx.send(value).unwrap();
        };

        let task = Arc::new(Task {
            id: AtomicUsize::new(0),
            future: Mutex::new(Box::pin(fut)),
        });

        let executor = Self::get();

        let mut run_q = executor.run_q.lock().unwrap();
        run_q.push(task);

        executor.cv.notify_all();

        TaskJoiner { rx }
    }

    pub fn block_on<Fut, T>(f: Fut) -> T
    where
        Fut: Future<Output = T> + Sync + Send + 'static,
        T: Send + 'static,
    {
        Self::spawn(f).join()
    }

    fn executor_loop() -> ! {
        let executor = Self::get();

        loop {
            let task = {
                let run_q = executor.run_q.lock().unwrap();

                let mut run_q = executor.cv.wait_while(run_q, |q| q.is_empty()).unwrap();

                run_q.pop().unwrap()
            };

            let task = {
                let mut waiting = executor.waiting.lock().unwrap();

                let slot = waiting.vacant_entry();

                task.id.store(slot.key(), Ordering::Relaxed);

                slot.insert(task.clone());

                task
            };

            let waker = Waker::from(task.clone());
            let mut cx = Context::from_waker(&waker);

            let mut fut = task.future.lock().unwrap();

            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(()) => {
                    let mut waiting = executor.waiting.lock().unwrap();
                    waiting.remove(task.id.load(Ordering::Relaxed));
                }
                Poll::Pending => {}
            }
        }
    }
}
