# `trale`: Tiny Rust Async Linux Executor

This project implements a minimalistic asynchronous Rust executor, written in as
few lines as possible. Its primary goal is to serve as an educational resource
for those studying Rust's async ecosystem. It provides a *real* executor capable
of running multiple async tasks on a single thread, showcasing a simple yet
functional concrete implementation.

To achieve this, `trale` tightly integrates with Linux's `epoll` interface,
opting for minimal abstractions to prioritize performance. While it sacrifices
some abstraction in favor of efficiency, **correctness** is not compromised.

## Supported Features

- **`epoll`-based reactor**: Handles task wake-ups when events are fired.
- **Single-threaded executor**: Polls tasks on a runqueue and moves them to an
  idle queue when waiting for wakeups from the reactor.
- **Timer using `TimerFd`**: Leverages Linux's
  [`timerfd_create`](https://linux.die.net/man/2/timerfd_create).
- **UDP sockets**: Non-blocking `std::net::UdpSocket` support.
- **TCP sockets**: Basic TCP socket support.
- **Inter-task events**: Uses [`EventFd`](https://linux.die.net/man/2/eventfd)
  for inter-task communication.
- **Task synchronization**: Implements synchronization via a `Mutex` type,
  backed by `EventFd` as the primitive.

Example Usage
-----

To see various examples, see the `examples/` directory in the root of
the project.  As a starting point:

```rust
use std::time::Duration;

use trale::{futures::timer::Timer, task::Executor};

fn main() {
    env_logger::init();

    Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).unwrap().await;
        println!("Hello A!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello B!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello C!");
    });

    Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).unwrap().await;
        println!("Hello a!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello b!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello c!");
    });

    Executor::run();
}

```

- `timer`: This example spawns two tasks which, both racing to print
  messages to the terminal.
- `udp`: This example transfers twenty bytes between two tasks usng
  UDP sockets on the localhost interface whilst a third task is
  printing messages to the terminal.
- `tcp`: This is an implementation of a TCP echo client (connecting to
  `127.0.0.1:5000`) whilst another task prints out messages to the
  terminal.
- `tcp_serv`: This is an implementation of a TCP echo server (waiting for
  connections to `127.0.0.1:5000`) whilst another task prints out messages to
  the terminal.
- `sub_tasks`: This demonstates how a subtask can be spawned from a 
   parent task and the joinhandle can be `.await`ed on without blocking
   the runqueue.

## Implementation

### Tasks & Executor

Each `Task` represents an asynchronous computation that needs to be executed. It
contains a top-level pinned and boxed future, referred to as the `future`, which
is typically an `async` block passed to the `spawn()` function. This function
creates a `Task` object and places it on the executor's per-thread run queue.

It is the responsibility of the `Executor` to call `poll()` on a task's
top-level future to advance its execution. Since futures are state machines,
each call to `poll()` modifies the future’s internal state on the heap. If
`poll()` returns `Poll::Pending`, the future will 'resume' execution from the
same point the next time `poll()` is called. This process is recursive: if a
future `await`s other futures, `poll()` will be called on those sub-futures as
well. The recursion continues until a "terminal" future is reached, which
typically interacts with the reactor to schedule a wakeup when execution can
proceed.

The `Executor` is responsible for pushing tasks to completion. It consists of a
run queue and a wait queue. When synchronous code calls either `Executor::run`
or `Executor::spawn_blocking`, the `Executor::executor_loop` function is invoked
on the same thread, orchestrating the execution of futures.

The execution loop follows these steps:

1. **Check for Tasks to Process**: The loop first checks if both the run queue
   and the wait queue are empty. If both are empty, the loop exits, as there are
   no more tasks to process.
2. **Remove a Task**: If there are tasks to process, a task is removed from the
   run queue.
   - If the run queue is empty, `epoll_wait` is invoked, via the reactor, to
     block the thread until a task becomes ready.
   - When `epoll_wait` returns, the corresponding `Waker` is invoked, which
     places the task back onto the run queue.
3. **Poll the Task**: The `Executor` calls `poll()` on the task's future to make
   progress.
   - If `poll()` returns `Poll::Ready`, the task is discarded as it has
     completed.
   - If `poll()` returns `Poll::Pending`, the task is placed in the wait queue
     for later processing.

### Reactor

The Reactor is responsible for invoking `Task::wake()` whenever a task can make
progress. This typically occurs when a file descriptor (FD) changes state, such
as when it becomes readable or writable. The Reactor's job is to associate a
task's `Waker` with an FD, detect changes in the FD's state, and trigger the
corresponding waker.

Terminal futures can register with the reactor via the `register_waker()`
function. This function takes a file descriptor (FD), a waker, and a flag
indicating whether to monitor the FD for read or write activity. Once a future
registers with the reactor, it should return `Poll::Pending` in order to place
the task back onto the wait queue (for example, in the case of a `Timer`
future).

The reactor has a `wait()` function, which is invoked when the runtime
determines that no other tasks can make progress. The `wait()` function calls
`epoll_wait()`, blocking until one or more monitored FDs change state. Once an
event is detected, the reactor maps the event to the associated waker and calls
`wake()` on it, which results in the task being placed back onto the run queue
for execution.
