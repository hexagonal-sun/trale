//! Async TCP Sockets.
//!
//! This module implements an async version of [std::net::TcpStream]. It allows
//! reception and transmission to and from TCP sockets to be defered by
//! `.await`ing the return values of [AsyncRead::read] and
//! [AsyncWrite::write].
//!
//! # Example
//!
//! Here is an example of an async tcp echo server.
//! ```no_run
//! use std::net::Ipv4Addr;
//! use trale::futures::tcp::TcpListener;
//! use trale::futures::read::AsyncRead;
//! use trale::futures::write::AsyncWrite;
//! async {
//!     let listener = TcpListener::bind("0.0.0.0:8888")?;
//!     let mut sock = listener.accept().await?;
//!     let mut buf = [0u8; 1];
//!     loop {
//!         let len = sock.read(&mut buf).await?;
//!         if len == 0 {
//!             return Ok(());
//!         }
//!         sock.write(&buf).await?;
//!     }
//!#     Ok::<(), std::io::Error>(())
//! };
//! ```
use std::{
    future::Future,
    io::{self, ErrorKind},
    net::{SocketAddr, ToSocketAddrs},
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
    pin::Pin,
    ptr::null_mut,
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use libc::{AF_INET, AF_INET6, EALREADY, EINPROGRESS, O_NONBLOCK, SOCK_NONBLOCK, SOCK_STREAM};

use crate::reactor::{Reactor, WakeupKind};

use super::{
    read::{AsyncRead, AsyncReader},
    write::{AsyncWrite, AsyncWriter},
};

/// A socket that is listening for incoming connections.
///
/// Use the [TcpListener::bind] function to obtain a listener that is ready to
/// accept connections on the specified address.
pub struct TcpListener {
    inner: OwnedFd,
}

/// A future for accepting new connections.
///
/// Call `.await` to pause the current task until a new connection has been
/// established.
pub struct Acceptor<'fd> {
    inner: BorrowedFd<'fd>,
}

fn mk_sock(addr: &SocketAddr) -> std::io::Result<OwnedFd> {
    let family = if addr.is_ipv4() { AF_INET } else { AF_INET6 };

    let sock = unsafe { libc::socket(family, SOCK_STREAM | SOCK_NONBLOCK, 0) };

    if sock == -1 {
        Err(std::io::Error::last_os_error())?;
    }

    let sock = unsafe { OwnedFd::from_raw_fd(sock) };

    Ok(sock)
}

impl TcpListener {
    /// Bind a new socket and return a listener.
    ///
    /// This function will create a new socket, bind it to one of the specified
    /// `addrs` and returna [TcpListener]. If binding to *all* of the specified
    /// addresses fails then the reason for failing to bind to the *last*
    /// address is returned. Otherwise, use [TcpListener::accept] to obtain a
    /// future to accept new connections.
    pub fn bind(addrs: impl ToSocketAddrs) -> std::io::Result<Self> {
        let addrs = addrs.to_socket_addrs()?;
        let mut last_err = ErrorKind::NotFound.into();

        for addr in addrs {
            let sock = mk_sock(&addr)?;
            let caddr: CSockAddr = addr.into();

            if unsafe { libc::bind(sock.as_raw_fd(), caddr.as_ptr(), caddr.len as _) } == -1 {
                last_err = std::io::Error::last_os_error();
                continue;
            }

            match unsafe { libc::listen(sock.as_raw_fd(), 1024) } {
                -1 => last_err = std::io::Error::last_os_error(),
                0 => return Ok(Self { inner: sock }),
                _ => unreachable!("listen() cannot return a value other than 0 or -1"),
            }
        }

        Err(last_err)
    }

    /// Return a future for accepting a new connection.
    ///
    /// You can `.await` the returned future to wait for a new incoming
    /// connection. Once a connection has been successfully established, a new
    /// [TcpStream] is returned which is connected to the peer.
    pub fn accept(&self) -> Acceptor {
        Acceptor {
            inner: self.inner.as_fd(),
        }
    }
}

impl Future for Acceptor<'_> {
    type Output = std::io::Result<TcpStream>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let ret =
            unsafe { libc::accept4(self.inner.as_raw_fd(), null_mut(), null_mut(), O_NONBLOCK) };

        if ret != -1 {
            return Poll::Ready(Ok(TcpStream {
                inner: unsafe { OwnedFd::from_raw_fd(ret) },
            }));
        }

        let err = std::io::Error::last_os_error();

        match err.raw_os_error().unwrap() {
            libc::EWOULDBLOCK => {
                Reactor::register_waker(
                    self.inner.as_fd(),
                    cx.waker().clone(),
                    WakeupKind::Readable,
                );

                Poll::Pending
            }
            _ => Poll::Ready(Err(err)),
        }
    }
}

/// A connection to a peer via the TCP.
///
/// There are two ways to obtain a connection, either through
/// [TcpListener::bind] to wait for incoming connections, or attempt to
/// establish a new connection with [TcpStream::connect]. This type implements
/// the [AsyncRead] and [AsyncWrite] traits to read and write from the socket.
pub struct TcpStream {
    inner: OwnedFd,
}

impl TcpStream {
    /// Attempt to connect to a peer
    ///
    /// This function will attempt to connect to the specified addresses via
    /// TCP. If a more than one address is specified, then the first successful
    /// connection that was able to be established is returned. If connection to
    /// all the addresses failed, then the reason the failture for the *last*
    /// address is returned.
    pub async fn connect<A: ToSocketAddrs>(addrs: A) -> Result<Self> {
        let addrs = addrs.to_socket_addrs()?;
        let mut last_err: anyhow::Error = anyhow!("Could not get socket addr");

        for addr in addrs {
            let sock = mk_sock(&addr)?;

            let connect = SockConnect {
                fd: sock.as_fd(),
                addr,
            };

            match connect.await {
                Ok(()) => return Ok(Self { inner: sock }),
                Err(e) => last_err = e.into(),
            }
        }

        Err(last_err)
    }
}

struct SockConnect<'fd> {
    fd: BorrowedFd<'fd>,
    addr: SocketAddr,
}

union CSockAddrs {
    v4: libc::sockaddr_in,
    v6: libc::sockaddr_in6,
}

struct CSockAddr {
    addr: CSockAddrs,
    len: usize,
}

impl From<SocketAddr> for CSockAddr {
    fn from(value: SocketAddr) -> Self {
        match value {
            SocketAddr::V4(addr) => {
                let mut sin_addr: CSockAddrs = unsafe { std::mem::zeroed() };
                sin_addr.v4.sin_family = AF_INET as u16;
                sin_addr.v4.sin_addr.s_addr = libc::htonl(addr.ip().to_bits());
                sin_addr.v4.sin_port = libc::htons(addr.port());

                CSockAddr {
                    addr: sin_addr,
                    len: std::mem::size_of::<libc::sockaddr_in>(),
                }
            }
            SocketAddr::V6(addr) => {
                let mut sin_addr: CSockAddrs = unsafe { std::mem::zeroed() };
                sin_addr.v6.sin6_family = AF_INET as u16;
                sin_addr.v6.sin6_addr.s6_addr = addr.ip().octets();
                sin_addr.v6.sin6_port = libc::htons(addr.port());

                CSockAddr {
                    addr: sin_addr,
                    len: std::mem::size_of::<libc::sockaddr_in6>(),
                }
            }
        }
    }
}

impl CSockAddr {
    fn as_ptr(&self) -> *const libc::sockaddr {
        &self.addr as *const _ as *const _
    }
}

impl Future for SockConnect<'_> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let addr: CSockAddr = self.addr.into();

        let ret = unsafe { libc::connect(self.fd.as_raw_fd(), addr.as_ptr(), addr.len as _) };

        if ret == 0 {
            return Poll::Ready(Ok(()));
        }

        let err = std::io::Error::last_os_error();

        match err.raw_os_error().unwrap() {
            EINPROGRESS | EALREADY => {
                Reactor::register_waker(
                    self.fd.as_raw_fd(),
                    cx.waker().clone(),
                    WakeupKind::Writable,
                );
                Poll::Pending
            }
            _ => Poll::Ready(Err(err)),
        }
    }
}

impl AsyncRead for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>> {
        AsyncReader {
            fd: self.inner.as_fd(),
            buf,
        }
    }
}

impl AsyncWrite for TcpStream {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>> {
        AsyncWriter {
            fd: self.inner.as_fd(),
            buf,
        }
    }
}
