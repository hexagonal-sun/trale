//! Async UDP Sockets.
//!
//! This module implements an async version of [std::net::UdpSocket].
//! It allows reception and transmission to and from UDP sockets to
//! be defered by `.await`ing the return values of
//! [UdpSocket::recv_from] and [UdpSocket::send_to].
//!
//! # Example
//!
//! Here is an example of an async UDP echo server.
//! ```no_run
//! use std::net::Ipv4Addr;
//! use trale::futures::udp::UdpSocket;
//! async {
//!     let mut sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 8888))?;
//!     let mut buf = [0u8; 1500];
//!     loop {
//!         let (len, src) = sock.recv_from(&mut buf).await?;
//!         sock.send_to(&buf[..len], src).await?;
//!     }
//!#     Ok::<(), std::io::Error>(())
//! };
//! ```

use std::future::Future;
use std::io;
use std::io::Result;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::net::UdpSocket as StdUdpSocket;
use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use crate::reactor::Reactor;
use crate::reactor::WakeupKind;

/// An async datagram socket.
pub struct UdpSocket {
    inner: StdUdpSocket,
}

impl UdpSocket {
    /// Attempt to bind to the local address `A` and return a new
    /// [UdpSocket].  [Self::recv_from] and [Self::send_to] can then
    /// be called on this socket once bound.
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let sock = StdUdpSocket::bind(addr)?;

        sock.set_nonblocking(true)?;

        Ok(Self { inner: sock })
    }

    /// Wait for reception of a datagram.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::net::Ipv4Addr;
    /// use trale::futures::udp::UdpSocket;
    /// async {
    ///     let mut sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    ///     let mut buf = [0u8; 1500];
    ///     let (len, src) = sock.recv_from(&mut buf).await?;
    ///#     Ok::<(), std::io::Error>(())
    /// };
    /// ```
    pub fn recv_from<'a>(&'a mut self, buf: &'a mut [u8]) -> RecvFrom {
        RecvFrom {
            sock: &self.inner,
            buf,
        }
    }

    /// Wait for reception of a datagram.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::net::Ipv4Addr;
    /// use trale::futures::udp::UdpSocket;
    /// async {
    ///     let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    ///     let buf = [0xad, 0xbe, 0xef];
    ///     sock.send_to(&buf, (Ipv4Addr::new(192, 168, 0, 1), 8080)).await?;
    ///#     Ok::<(), std::io::Error>(())
    /// };
    /// ```
    pub fn send_to<'a, A: ToSocketAddrs>(&'a self, buf: &'a [u8], target: A) -> SendTo<A> {
        SendTo {
            sock: &self.inner,
            dst: target,
            buf,
        }
    }
}

/// A future that receives data into a datagram socket, see
/// [UdpSocket::recv_from].
pub struct RecvFrom<'a, 'b> {
    sock: &'a StdUdpSocket,
    buf: &'b mut [u8],
}

impl Future for RecvFrom<'_, '_> {
    type Output = Result<(usize, SocketAddr)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.sock.recv_from(self.buf) {
            Ok(ret) => Poll::Ready(Ok(ret)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                Reactor::register_waker(
                    self.sock.as_raw_fd(),
                    cx.waker().clone(),
                    WakeupKind::Readable,
                );

                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

/// A future that send data from a datagram socket, see
/// [UdpSocket::send_to].
pub struct SendTo<'a, 'b, A: ToSocketAddrs> {
    sock: &'a StdUdpSocket,
    dst: A,
    buf: &'b [u8],
}

impl<A: ToSocketAddrs> Future for SendTo<'_, '_, A> {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.sock.send_to(self.buf, &self.dst) {
            Ok(ret) => Poll::Ready(Ok(ret)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                Reactor::register_waker(
                    self.sock.as_raw_fd(),
                    cx.waker().clone(),
                    WakeupKind::Writable,
                );

                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UdpSocket;
    use crate::task::Executor;
    use std::net::Ipv4Addr;

    #[test]
    fn send_recv() {
        Executor::block_on(async {
            let dst = (Ipv4Addr::LOCALHOST, 8086);
            let tx_sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let mut rx_sock = UdpSocket::bind(dst).unwrap();

            let task = Executor::spawn(async move {
                let mut buf = [0; 4];
                rx_sock.recv_from(&mut buf).await.unwrap();
            });

            tx_sock
                .send_to(&0xdeadbeef_u32.to_le_bytes(), dst)
                .await
                .unwrap();

            task.await;
        });
    }
}
