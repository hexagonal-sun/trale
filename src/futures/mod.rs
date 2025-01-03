//! Future sub-modules.
//!
//! The `futures` module serves as the foundation for various types of futures
//! used within the `trale` executor. It encapsulates a collection of
//! sub-modules, each implementing a different kind of future. These futures are
//! crucial building blocks for the system, as they represent computations that
//! the reactor can sleep on while waiting for events or conditions to change.
//!
//! Each future type is designed to handle specific aspects of asynchronous
//! execution, such as file descriptor monitoring, timers, or inter-task
//! synchronization. The reactor interacts with these futures to determine when
//! tasks can make progress, allowing the executor to efficiently manage
//! multiple tasks concurrently.
//!
//! The following sub-modules are exposed by the `futures` module:
//!
//! - `event`: Provides futures for inter-task event signaling.
//! - `mutex`: Implements futures for task synchronization using a mutex-like primitive.
//! - `read`: Implements futures for reading from non-blocking file descriptors.
//! - `tcp`: Provides futures for handling TCP socket operations.
//! - `timer`: Implements futures for timer-based tasks using `timerfd`.
//! - `udp`: Provides futures for handling UDP socket operations.
//! - `write`: Implements futures for writing to non-blocking file descriptors.
//!
//! Together, these futures form the core of the `trale` executor's
//! functionality, enabling the reactor to monitor and interact with various
//! asynchronous operations.
pub mod event;
pub mod mutex;
pub mod read;
pub mod tcp;
pub mod timer;
pub mod udp;
pub mod write;
