# `trale`: Tiny Rust Async Linux Executor

This project implements a minimalistic asynchronous Rust executor, written in as
few lines as possible. Its primary goal is to serve as an educational resource
for those studying Rust's async ecosystem. It provides a *real* executor capable
of running multiple async tasks on a single thread, showcasing a simple yet
functional concrete implementation.

To achieve this, `trale` tightly integrates with Linux's `io_uring` interface,
opting for minimal abstractions to prioritise performance. While it sacrifices
some abstraction in favour of efficiency, **correctness** is not compromised.

## Supported Features

- **`io_uring`-based reactor**: State-of-the-art reactor utilising Linux's
  latest I/O userspace
  interface,[`io_uring`](https://man7.org/linux/man-pages/man7/io_uring.7.html).
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
   - If the run queue is empty, `io_uring_enter` is invoked, via the reactor, to
     block the thread until an I/O event completes.
   - When `io_uring_enter` returns, the corresponding all I/O events that have
     completed have their corresponding `Waker`s called which places the task on
     the runqueue.
3. **Poll the Task**: The `Executor` calls `poll()` on the task's future to make
   progress.
   - If `poll()` returns `Poll::Ready`, the task is discarded as it has
     completed.
   - If `poll()` returns `Poll::Pending`, the task is placed in the wait queue
     for later processing.

### Reactor

The Reactor is responsible for invoking `Task::wake()` whenever a task can make
progress. This typically occurs when some I/O operation completes. Each future
that requires I/O interaction needs to obtain a handle to the reactor's I/O
interface via `Reactor::new_io()`. This handle, represented by `ReactorIo`, is
used for submitting I/O operations and retrieving their results. The key
function for interacting with the reactor is `submit_or_get_result()`. This
function is designed to be called within the `poll()` method of a future,
providing a bridge between the future and the reactor.

The `submit_or_get_result()` function takes a closure that is responsible for
creating an I/O operation, which is encapsulated in an `Entry`, and associates
it with a `Waker` to notify the task when the operation has completed. The
`Entry` describes the type of I/O operation, such as a read or write, and
contains the necessary arguments to be passed to the kernel. The `Waker` is used
to wake the task once the I/O operation is ready to be processed.

One of the most important characteristics of this system is that the closure
provided to `submit_or_get_result()` is invoked only once to submit the I/O
request to the kernel. This design isn't just a performance optimisation; it
also addresses soundness concerns. Since buffers and other resources are shared
between the user space and the kernel, submitting the same I/O operation
multiple times could lead to serious issues. For instance, if a future were
polled more than once and the I/O request were re-submitted, the original
submission might contain references to deallocated memory, invalid file
descriptors, or other corrupted state. By ensuring that the closure is only
called once, we avoid these potential pitfalls. On the first call, the function
returns `Poll::Pending`, placing the task in a pending state until the operation
completes. If the task is polled again before the I/O has finished, it simply
returns `Poll::Pending` without invoking the closure, as the reactor already
knows about the pending I/O operation. Once the I/O completes,
`submit_or_get_result()` returns `Poll::Ready` with the result of the I/O
operation, encapsulated in a `std::io::Result<i32>`.
