//! Task and execution management
//!
//! This module provides the methods needed to spawn tasks and execute them
//! until completion. Trale uses a per-thread exector model which means that
//! each new OS thread that is spawned has it's own execution environemnt. This
//! means that:
//!
//! 1. The thread upon which a task is spawned is the same thread that will
//!    execute it.
//! 2. Each thread needs to call one of [Executor::block_on] or [Executor::run]
//!    to do any work. Use the former if you want to run a single top-level
//!    task, or the latter if you wish to run multiple top-level tasks.
//!
//! # Example
//!
//! Here is a simple hello world using [Executor::block_on].
//!
//! ```
//! use trale::task::Executor;
//! Executor::block_on(async { println!("Hello, world!"); });
//! ```
//!
//! You can also use [Executor::block_on] to easily obtain the result of a
//! future:
//!
//! ```
//! use trale::task::Executor;
//! let x = Executor::block_on(async { 2 + 8 });
//! assert_eq!(x, 10);
//! ```
//!
//! Here is the same but spawning multiple top-level tasks.
//!
//! ```
//! use trale::task::Executor;
//! Executor::spawn(async { println!("Hello"); });
//! Executor::spawn(async { println!("World!"); });
//! Executor::run();
//! ```
//!
//! # Threading Model
//!
//! Since each thread has it's own execution state, if you don't spawn any new
//! threads you can guarantee that only a single task will be executing at once.
//! This allows for `!Sync` futures to be executed:
//!
//! ```
//! use trale::task::Executor;
//! use std::cell::RefCell;
//! use std::rc::Rc;
//! let cell = Rc::new(RefCell::new(0));
//! {
//!     let cell = cell.clone();
//!     Executor::spawn(async move { *cell.borrow_mut() += 10 });
//! }
//! {
//!     let cell = cell.clone();
//!     Executor::spawn(async move { *cell.borrow_mut() += 10 });
//! }
//! Executor::run();
//! assert_eq!(*cell.borrow(), 20);
//! ```
//!
//! Note, however, that one needs to call [Executor::run] for *all* thread that
//! need to execute futures:
//! ```
//! use trale::task::Executor;
//! use std::sync::Arc;
//! use std::sync::Mutex;
//! use std::thread;
//! let cell = Arc::new(Mutex::new(0));
//! {
//!     let cell = cell.clone();
//!     Executor::spawn(async move { *cell.lock().unwrap() += 1 });
//! }
//! {
//!     let cell = cell.clone();
//!     Executor::spawn(async move { *cell.lock().unwrap() += 1 });
//! }
//!
//! let thread = {
//!     let cell = cell.clone();
//!     thread::spawn(move || {
//!         Executor::spawn(async move { *cell.lock().unwrap() += 1 });
//!         Executor::run();
//!     })
//! };
//!
//! thread.join();
//! assert_eq!(*cell.lock().unwrap(), 1);
//! Executor::run();
//! assert_eq!(*cell.lock().unwrap(), 3);
//! ```
use std::{
    cell::RefCell,
    future::Future,
    mem::transmute,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{sync_channel, Receiver},
        Arc,
    },
    task::{ready, Context, Poll, Wake, Waker},
};

use slab::Slab;

use crate::{
    futures::event::{Event, EventWaiter},
    reactor::Reactor,
};

struct TaskId(AtomicUsize);

impl Wake for TaskId {
    fn wake(self: Arc<TaskId>) {
        EXEC.with(|exec| {
            let mut exec = exec.borrow_mut();
            if let Some(task) = exec.waiting.try_remove(self.0.load(Ordering::Relaxed)) {
                exec.run_q.push(task);
            }
        });
    }
}

struct Task {
    id: Arc<TaskId>,
    future: Pin<Box<dyn Future<Output = ()>>>,
}

/// The async executor.
///
/// A type that is responsible for pushing futures through to
/// completion. You can begin execution of a new task by calling the
/// [Executor::block_on] function.
pub struct Executor {
    waiting: Slab<Task>,
    run_q: Vec<Task>,
}

thread_local! {
    static EXEC: RefCell<Executor> = const { RefCell::new(
        Executor {
            waiting: Slab::new(),
            run_q: Vec::new(),
        }
    )}
}

/// A handle to a running task.
///
/// You can call [TaskJoiner::join] from a synchronous context to block
/// execution and yield the future's value. If you want to wait for execution to
/// finish from an asynchronous context, use `.await` on the joiner. If the
/// joiner is dropped then execution of the future continues to completion but
/// the return value is lost, aka detatch-on-drop.
pub struct TaskJoiner<'a, T> {
    rx: Receiver<T>,
    _evt: Event,
    finished: EventWaiter<'a>,
}

impl<'a, T> TaskJoiner<'a, T> {
    /// Block execution and wait for a task to finish executing. The return
    /// value `T` is the value yielded by the task's future.
    ///
    /// *Note* This function should only be called from synchronous contexts. To
    /// prevent deadlocks in an asynchronous context, use `.await` instead.
    pub fn join(self) -> T {
        self.rx.recv().unwrap()
    }
}

impl<'a, T> Future for TaskJoiner<'a, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        ready!(Pin::new(&mut self.finished).poll(cx)).unwrap();

        Poll::Ready(self.rx.recv().unwrap())
    }
}

impl Executor {
    /// Spawn a new future and add it to this thread's run queue. If called from
    /// an already-running asynchronous task, the future will be queued for
    /// execution. If called from a synchronous context, the task will *not* be
    /// executed until [Executor::run] is called.
    ///
    /// A [TaskJoiner] is returned which can be used to wait for completion of
    /// the future `f` and obtain it's return value.
    pub fn spawn<'a, Fut, T>(f: Fut) -> TaskJoiner<'a, T>
    where
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = sync_channel(1);
        let mut evt = Event::new().unwrap();
        let evt2 = evt.clone();

        let fut = async move {
            let value = f.await;
            let _ = evt2.notify_one();
            let _ = tx.send(value);
        };

        let task = Task {
            id: Arc::new(TaskId(AtomicUsize::new(0))),
            future: Box::pin(fut),
        };

        EXEC.with(|exec| {
            exec.borrow_mut().run_q.push(task);
        });

        // SAFETY: This is safe since the borrowed FD is in the same structure
        // that contains the waiter, therefore waiter can never outlive the
        // event.
        let waiter: EventWaiter<'static> = unsafe { transmute(evt.wait()) };

        TaskJoiner {
            rx,
            _evt: evt,
            finished: waiter,
        }
    }

    /// A convenience function for waiting on a future from a synchronous
    /// context. This is the equivalent of calling:
    ///
    /// ```
    /// # use trale::task::Executor;
    /// # use std::future::Future;
    /// # fn x<Fut: Future<Output = ()> + Send + 'static>(f: Fut) {
    /// let task = Executor::spawn(f);
    /// Executor::run();
    /// task.join();
    /// # }
    /// ```
    pub fn block_on<Fut, T>(f: Fut) -> T
    where
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        let joiner = Self::spawn(f);

        Self::executor_loop();

        joiner.join()
    }

    /// Run the executor for this thread.
    ///
    /// This function will schedule and run all tasks that have been previously
    /// spawned with [Executor::spawn]. *Note* each thread has it's own set of
    /// tasks and execution envionrment. If you call this function, only tasks
    /// that have been spaned on *this* thread will be executed.
    ///
    /// Blocks until all tasks have finished executing.
    pub fn run() {
        Self::executor_loop()
    }

    fn executor_loop() {
        EXEC.with(|exec| loop {
            if exec.borrow().run_q.is_empty() {
                Reactor::react();
            }

            let mut task = exec.borrow_mut().run_q.pop().unwrap();

            let waker = Waker::from(task.id.clone());

            let mut cx = Context::from_waker(&waker);

            match task.future.as_mut().poll(&mut cx) {
                Poll::Ready(()) => {}
                Poll::Pending => {
                    let waiting = &mut exec.borrow_mut().waiting;

                    let slot = waiting.vacant_entry();

                    task.id.0.store(slot.key(), Ordering::Relaxed);

                    slot.insert(task);
                }
            }

            if exec.borrow().run_q.is_empty() && exec.borrow().waiting.is_empty() {
                return;
            }
        });
    }
}
