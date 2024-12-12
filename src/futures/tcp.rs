use std::{
    future::Future,
    io,
    net::ToSocketAddrs,
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use nix::{
    errno::Errno,
    sys::socket::{connect, socket, AddressFamily, SockFlag, SockType, SockaddrStorage},
};

use crate::reactor::{Reactor, WakeupKind};

use super::{
    read::{AsyncRead, AsyncReader},
    write::{AsyncWrite, AsyncWriter},
};

pub struct TcpStream {
    inner: OwnedFd,
}

impl TcpStream {
    pub async fn connect<A: ToSocketAddrs>(addrs: A) -> Result<Self> {
        let addrs = addrs.to_socket_addrs()?;
        let mut last_err: anyhow::Error = anyhow!("Could not get socket addr");

        for addr in addrs {
            let family = if addr.is_ipv4() {
                AddressFamily::Inet
            } else {
                AddressFamily::Inet6
            };

            let sock = socket(
                family,
                SockType::Stream,
                SockFlag::SOCK_NONBLOCK,
                None,
            )?;

            let connect = SockConnect {
                fd: sock.as_fd(),
                addr: addr.into(),
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
    addr: SockaddrStorage,
}

impl Future for SockConnect<'_> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match connect(self.fd.as_raw_fd(), &self.addr) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(Errno::EINPROGRESS) => {
                Reactor::get().register_waker(
                    self.fd.clone(),
                    cx.waker().clone(),
                    WakeupKind::Writable,
                );
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e.into())),
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
