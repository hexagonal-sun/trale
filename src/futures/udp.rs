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

use std::{
    future::Future,
    io::Result,
    net::{SocketAddr, ToSocketAddrs, UdpSocket as StdUdpSocket},
    os::fd::AsRawFd,
    pin::Pin,
    task::{Context, Poll},
};

use io_uring::{opcode, types};
use libc::{iovec, msghdr};

use crate::reactor::{Reactor, ReactorIo};

use super::sock_addr::CSockAddr;

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
            io: Reactor::new_io(),
            hdr: unsafe { std::mem::zeroed() },
            iov: unsafe { std::mem::zeroed() },
            csock: unsafe { std::mem::zeroed() },
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
            io: Reactor::new_io(),
            buf,
            hdr: unsafe { std::mem::zeroed() },
            csock: unsafe { std::mem::zeroed() },
            iov: unsafe { std::mem::zeroed() },
        }
    }
}

/// A future that receives data into a datagram socket, see
/// [UdpSocket::recv_from].
pub struct RecvFrom<'a, 'b> {
    sock: &'a StdUdpSocket,
    io: ReactorIo,
    hdr: msghdr,
    iov: iovec,
    csock: CSockAddr,
    buf: &'b mut [u8],
}

impl Future for RecvFrom<'_, '_> {
    type Output = Result<(usize, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        this.io
            .submit_or_get_result(|| {
                this.iov.iov_base = this.buf.as_mut_ptr() as *mut _;
                this.iov.iov_len = this.buf.len();
                this.hdr.msg_iov = &mut this.iov as *mut _;
                this.hdr.msg_iovlen = 1;
                this.hdr.msg_name = &mut this.csock.addr as *mut _ as *mut _;
                this.hdr.msg_namelen = std::mem::size_of_val(&this.csock.addr) as _;

                (
                    opcode::RecvMsg::new(types::Fd(this.sock.as_raw_fd()), &mut this.hdr as *mut _)
                        .build(),
                    cx.waker().clone(),
                )
            })
            .map(|x| {
                let sz = x?;
                this.csock.len = this.hdr.msg_namelen as _;

                match <&CSockAddr as TryInto<SocketAddr>>::try_into(&this.csock) {
                    Ok(addr) => Ok((sz as _, addr)),
                    Err(e) => Err(e),
                }
            })
    }
}

/// A future that send data from a datagram socket, see
/// [UdpSocket::send_to].
pub struct SendTo<'a, 'b, A: ToSocketAddrs> {
    sock: &'a StdUdpSocket,
    dst: A,
    io: ReactorIo,
    hdr: msghdr,
    csock: CSockAddr,
    iov: iovec,
    buf: &'b [u8],
}

impl<A: ToSocketAddrs> Future for SendTo<'_, '_, A> {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        this.io
            .submit_or_get_result(|| {
                this.csock = this.dst.to_socket_addrs().unwrap().next().unwrap().into();
                this.hdr.msg_namelen = this.csock.len as _;
                this.hdr.msg_name = &mut this.csock.addr as *mut _ as *mut _;
                this.iov.iov_base = this.buf.as_ptr() as *mut _;
                this.iov.iov_len = this.buf.len();
                this.hdr.msg_iov = &mut this.iov as *mut _ as *mut _;
                this.hdr.msg_iovlen = 1;

                (
                    opcode::SendMsg::new(types::Fd(this.sock.as_raw_fd()), &this.hdr as *const _)
                        .build(),
                    cx.waker().clone(),
                )
            })
            .map(|x| x.map(|x| x as _))
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
