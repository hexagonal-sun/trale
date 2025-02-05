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

use io_uring::{opcode, types};
use libc::{AF_INET, AF_INET6, SOCK_STREAM};

use crate::reactor::{Reactor, ReactorIo};

use super::{
    read::{AsyncRead, AsyncReader},
    sock_addr::CSockAddr,
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
    io: ReactorIo,
}

fn mk_sock(addr: &SocketAddr) -> std::io::Result<OwnedFd> {
    let family = if addr.is_ipv4() { AF_INET } else { AF_INET6 };

    let sock = unsafe { libc::socket(family, SOCK_STREAM, 0) };

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
            io: Reactor::new_io(),
        }
    }
}

impl Future for Acceptor<'_> {
    type Output = std::io::Result<TcpStream>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let entry = opcode::Accept::new(types::Fd(self.inner.as_raw_fd()), null_mut(), null_mut());

        self.io
            .submit_or_get_result(|| (entry.build(), cx.waker().clone()))
            .map(|x| {
                x.map(|fd| TcpStream {
                    inner: unsafe { OwnedFd::from_raw_fd(fd) },
                })
            })
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
    pub async fn connect<A: ToSocketAddrs>(addrs: A) -> std::io::Result<Self> {
        let addrs = addrs.to_socket_addrs()?;
        let mut last_err: std::io::Error = ErrorKind::InvalidData.into();

        for addr in addrs {
            let sock = mk_sock(&addr)?;

            let connect = SockConnect {
                fd: sock.as_fd(),
                io: Reactor::new_io(),
                addr,
            };

            match connect.await {
                Ok(()) => return Ok(Self { inner: sock }),
                Err(e) => last_err = e,
            }
        }

        Err(last_err)
    }
}

struct SockConnect<'fd> {
    fd: BorrowedFd<'fd>,
    io: ReactorIo,
    addr: SocketAddr,
}

impl Future for SockConnect<'_> {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let addr: CSockAddr = self.addr.into();
        let entry =
            opcode::Connect::new(types::Fd(self.fd.as_raw_fd()), addr.as_ptr(), addr.len as _);

        self.io
            .submit_or_get_result(|| (entry.build(), cx.waker().clone()))
            .map(|x| x.map(|_| ()))
    }
}

impl AsyncRead for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>> {
        AsyncReader {
            fd: self.inner.as_fd(),
            io: Reactor::new_io(),
            buf,
        }
    }
}

impl AsyncWrite for TcpStream {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>> {
        AsyncWriter {
            fd: self.inner.as_fd(),
            io: Reactor::new_io(),
            buf,
        }
    }
}
