//! # `trale`: Tiny Rust Async Linux Executor
//!
//! This project implements a minimalistic asynchronous Rust executor, written
//! in as few lines as possible. Its primary goal is to serve as an educational
//! resource for those studying Rust's async ecosystem. It provides a *real*
//! executor capable of running multiple async tasks on a single thread,
//! showcasing a simple yet functional concrete implementation.
//!
//! To achieve this, `trale` tightly integrates with Linux's `epoll` interface,
//! opting for minimal abstractions to prioritize performance. While it
//! sacrifices some abstraction in favor of efficiency, **correctness** is not
//! compromised.
//!
//! For information about spawning and managing tasks, refer to the [task]
//! module. To see what futures are provided, see the [futures] module.
//!
//! ## Example
//!
//! This is a simple example of printing out `Hello, world!` based on timers:
//!
//! ```
//! use trale::futures::timer::Timer;
//! use trale::task::Executor;
//! use std::time::{Duration, Instant};
//! Executor::spawn( async {
//!     Timer::sleep(Duration::from_secs(1)).unwrap().await;
//!     println!("Hello, ");
//! });
//! Executor::spawn( async {
//!     Timer::sleep(Duration::from_secs(2)).unwrap().await;
//!     println!("World!");
//! });
//! Executor::run();
//! ```
pub mod futures;
pub(crate) mod reactor;
pub mod task;
