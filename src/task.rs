use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{sync_channel, Receiver},
        Arc, Condvar, Mutex,
    },
    task::{ready, Context, Poll, Wake, Waker},
    thread,
};

use async_lock::OnceCell;
use slab::Slab;

use crate::futures::event::Event;

struct Task {
    id: AtomicUsize,
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send>>>,
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

/// A type that is responsible for pushing futures through to
/// completion. You can begin execution of a new task by calling the
/// [Executor::spawn] function.
pub struct Executor {
    waiting: Mutex<Slab<Arc<Task>>>,
    run_q: Mutex<Vec<Arc<Task>>>,
    cv: Condvar,
}

/// This type represents a handle to a running task.  You can call
/// [TaskJoiner::join] from a synchronous context to block execution
/// and yield the future's value.  If you want to wait for execution
/// to finish from an asynchronous context, use `.await` on the
/// joiner.  If the joiner is dropped then execution of the future
/// continues to completion but the return value is lost.
pub struct TaskJoiner<T> {
    rx: Receiver<T>,
    finished: Event,
}

impl<T> TaskJoiner<T> {
    /// block execution and wait for a task to finish executing.  The
    /// return value `T` is the value yielded by the task's future.
    pub fn join(self) -> T {
        self.rx.recv().unwrap()
    }
}

impl<T> Future for TaskJoiner<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut waiter = self.finished.wait();

        ready!(Pin::new(&mut waiter).poll(cx)).unwrap();

        Poll::Ready(self.rx.recv().unwrap())
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

    /// Spawn a new future and begin executing it immediately.  A
    /// [TaskJoiner] is returned which can be used to wait for
    /// completion of the future `f` and obtain it's return value.
    pub fn spawn<Fut, T>(f: Fut) -> TaskJoiner<T>
    where
        Fut: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = sync_channel(1);
        let evt = Event::new().unwrap();
        let evt2 = evt.clone();

        let fut = async move {
            let value = f.await;
            evt2.notify_one().unwrap();
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

        TaskJoiner { rx, finished: evt }
    }

    /// A convenience function for waiting on a future from a
    /// synchronous context.  This is the equivalent of calling:
    ///
    /// ```
    /// # use trale::task::Executor;
    /// # use std::future::Future;
    /// # fn x<Fut: Future<Output = ()> + Send + 'static>(f: Fut) {
    /// Executor::spawn(f).join()
    /// # }
    /// ```
    pub fn block_on<Fut, T>(f: Fut) -> T
    where
        Fut: Future<Output = T> + Send + 'static,
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

            // It looks odd to put the task back on the waitqueue
            // before we call poll(). However this is needed to
            // prevent a race condition.  If `poll()` on the future
            // adds an fd to the reactor, that thread may attempt to
            // remove us from the waitqueue before this thread has
            // placed us in it.  It would reference an invalid `id`
            // this this would only be set *after* the call to poll.
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
