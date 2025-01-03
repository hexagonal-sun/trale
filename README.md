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
contains a top-level pinned-boxed future, called `future`, which is typically an
`async` block passed to the `spawn()` function. This function constructs a
`Task` object and places it on the executor's per-thread run-queue.

It is the responsibility of the `Executor` to call `poll()` on this top-level
task’s future to advance its execution. Since futures are state machines, each
call to `poll()` modifies the future’s internal state on the heap. If `poll()`
returns `Poll::Pending`, this allows the future to 'resume' execution at the
same point when `poll()` is called again. This same process applies to any
sub-futures that are `.await`ed: `poll()` is called on any sub-futures, which
may in turn call `poll()` on their sub-futures, continuing until a 'terminal'
future is reached. Typically, this terminal future will interact with the
reactor to schedule a wakeup when execution can proceed.

The `Executor` is responsible for pushing tasks to completion. It consists of a
run-queue, a wait-queue, and a thread that executes the `poll()` function for
the top-level futures. The only way to access the executor is via the
`Executor::get()` function, which ensures that only a single instance of the
executor is created, using a `OnceCell`.

When `Executor::get()` is called for the first time, a new thread is spawned to
execute `executor_loop()`. This loop performs the following:

1. It removes tasks from the run-queue (or waits if the run-queue is empty).
2. It places each task on the wait-queue, assuming it will return
   `Poll::Pending` on the first call to `poll()`.
3. It calls `poll()` on the task.
   - If `poll()` returns `Poll::Ready`, the task is removed from the wait-queue
     and discarded.
   - If `poll()` returns `Poll::Pending`, the task remains in the wait-queue,
     and the loop continues by picking the next task from the run-queue.

Whenever a task is ready to be polled again (as determined by the reactor), the
`Task::wake()` function is called. This removes the task from the wait-queue,
re-queues it onto the run-queue, and notifies the executor thread if it was
waiting for new tasks to be added to the run-queue.

### Reactor

The Reactor is responsible for invoking `Task::wake()` whenever a task can make
progress. This typically happens when a file descriptor (FD) changes state and
becomes readable or writable. The reactor’s job is to associate a task's waker
with an FD, detect changes on the FD, and trigger the corresponding waker.

Terminal futures can register with the reactor via the `register_waker()`
function. This function takes an FD, a waker, and a flag indicating whether to
monitor the FD for read or write activity. Once a future registers with the
reactor, it will return `Poll::Pending` in order to put the task back onto the
wait-queue (for example, see the `Timer` future).

The reactor consists of a separate thread that runs `reactor_loop()`. This loop
calls `epoll_wait()` repeatedly, which blocks until one or more of the monitored
FDs change state. Once an event is detected, the reactor maps the event back to
the associated waker and calls `wake()` on it, which results in the task being
placed back onto the run-queue.

