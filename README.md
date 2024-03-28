`trale`: Tiny Rust Async Linux Executor
=====

This project is an implementation of an asynchronous Rust executor,
written in as few a lines as possible. Its main purpose it to act
as a resource for people studying Rust's async implementation by
implementing a *real* executor that can execute multiple async tasks
on the same thread it showcases a simple, small concrete
implementation. To achieve this, we tightly couple with Linux's
`epoll` interface, facilitating abstraction omission and a high
tolerance for low performance (liberal use of `clone()` and heap
allocation).  However *correctness* isn't sacrificed.

*NOTE*: This project should not be used in production.  It's main
purpose is to be a learning aid.

Supported Features
-----

- An [`epoll`](https://linux.die.net/man/7/epoll) based reactor which
  handles waking up tasks when an event has fired.
- A simple single-threaded executor that polls tasks on a runqueue and
  stores them on a idle-queue when waiting for a wakeup from the
  reactor.
- A timer using Linux's [`TimerFd`](https://linux.die.net/man/2/timerfd_create).
- UDP sockets, using non-blocking `std::net::UdpSocket`.
- TCP sockets.

Example Usage
-----

To see various examples, see the `examples/` directory in the root of
the project.  As a starting point:

```rust
use std::time::Duration;
use trale::{task::Executor, timer::Timer};

fn main() {
    let task1 = Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).await;
        println!("Hello task 1!");
        1
    });

    let task2 = Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).await;
        println!("Hello task 2!");
        2
    });

    assert_eq!(task1.join(), 1);
    assert_eq!(task2.join(), 2);
}
```

- `timer`: This example spawns two tasks which, both racing to print
  messages to the terminal.
- `udp`: This example transfers twenty bytes between two tasks usng
  UDP sockets on the localhost interface whilst a third task is
  printing messages to the terminal.
- `tcp`: This is an implementation of a TCP echo server (connecting to
  `127.0.0.1:5000`) whilst another task prints out messages to the
  terminal.
- `sub_tasks`: This demonstates how a subtask can be spawned from a 
   parent task and the joinhandle can be `.await`ed on without blocking
   the runqueue.

## Implementation

### Tasks & Executor

Each `Task` represents a new asynchronous computation that we need to
execute.  It consists of a top-level pinned-boxed future, called
`future`, which is typically an `async` block, passed to the `spawn()`
function.  This function constructs a `Task` object and places it on
the executors run-queue.  It is the job of the `Executor` to call
`poll()` on this top-level task's future to push along execution.
Recall that futures are state-machines, therefore as we call `poll()`
on the top-level future we are modifying state on the heap.  If the
`poll()` function returns `Poll::Pending`, this state allows us to
'resume' execution from the same point the next time `poll()` is
called.  This is also the case for any sub-futures which are
`.await`ed: `poll()` is called on any sub-futures which call `poll()`
on their sub-futures and so forth until we hit a 'terminal' future.
This future will most likely call into the reactor to schedule a
wakeup once execution can progress.

The `Executor` object, as mentioned above, is responsible for pushing
tasks to their completion.  It consists of: a run-queue, a wait-queue
and a thread from which the top-level future's `poll()` function is
called.  The only way to get a reference to the executor is by calling
the `Executor::get()` function, ensuring that only a single instance
of the executor is created by using a `OnceCell`.  When this function
is called for the first time a new thread is spawned which calls
`executor_loop()`.  This loop removes any tasks from the run-queue (or
waits if the run-queue is empty), places the task on the wait-queue
(preempting the fact that the task will return `Poll::Pending`) and
then calls `poll()` on it.  If `poll()` returns `Poll::Ready`, the
task is removed from the wait-queue and is forgotten about, if the
task returns `Poll::Pending` we continue to pick off the next task
from the run-queue.

Whenever a task is ready to be `poll()`ed again (determined by the
reactor) the `Task::wake` function is called.  This removes the task
from the wait-queue, re-queues it back onto the run-queue and notifies
the executor thread in case it was sleeping waiting for a new task to
be added to the run-queue.

### Reactor

The Reactor is responsible for calling `Task::wake` whenever a task is
able to make progress.  Typically this is when a file-descriptor (FD)
changes state and is able to be read/written to.  It is the job of the
reactor to associate a task's waker and FD, detect changes on the FD
and call the corresponding waker.  Terminal futures can register with
the reactor via the `register_waker()` function.  This takes an FD,
waker, and whether to monitor for read or write activity on the FD.
Once a future has registered with the reactor they will then return
`Poll::Pending` to put the task back onto the wait-queue (see the
`Timer` future for an example).

The reactor also consists of a thread which calls `reactor_loop()`,
repeatedly calling `epoll_wait()`. This function returns when any of
the monitored FDs change state and allows us to map these events back
to a waker.  This thread will then call the `wake()` function for the
associated task, resulting in it being placed back onto the run-queue.
