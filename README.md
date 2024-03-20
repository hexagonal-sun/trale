`trale`: Tiny Rust Async Linux Executor
=====

This project aims to write a Rust async task executor without any
unsafe code and in as few a lines as possible.  To achieve this, we
tightly couple with Linux's `epoll` interface, allowing abstractions
to be omitted and don't worry too much about performance.

*NOTE*: This project should not be used in production.  It's main
purpose is to be a learning aid for someone trying to understand how
Rust's async system works.

Supported Features
-----

- An `epoll` based reactor which handles waking up tasks when an event
  has fired.
- A simple single-threaded executor that polls tasks on a runqueue and
  stores them on a idle-queue when waiting for a wakeup from the
  reactor.
- A timer using Linux's `TimerFd`.

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
