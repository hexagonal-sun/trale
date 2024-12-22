use std::{
    future::Future,
    io,
    net::{SocketAddr, ToSocketAddrs},
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use libc::{AF_INET, AF_INET6, EALREADY, EINPROGRESS, SOCK_NONBLOCK, SOCK_STREAM};

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
            let family = if addr.is_ipv4() { AF_INET } else { AF_INET6 };

            let sock = unsafe { libc::socket(family, SOCK_STREAM | SOCK_NONBLOCK, 0) };

            if sock == -1 {
                Err(std::io::Error::last_os_error())?;
            }

            let sock = unsafe { OwnedFd::from_raw_fd(sock) };

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

union CSockAddr {
    v4: libc::sockaddr_in,
    v6: libc::sockaddr_in6,
}

impl Future for SockConnect<'_> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (addr, len) = match self.addr {
            SocketAddr::V4(addr) => {
                let mut sin_addr: CSockAddr = unsafe { std::mem::zeroed() };
                sin_addr.v4.sin_family = AF_INET as u16;
                sin_addr.v4.sin_addr.s_addr = libc::htonl(addr.ip().to_bits());
                sin_addr.v4.sin_port = libc::htons(addr.port());

                (sin_addr, std::mem::size_of::<libc::sockaddr_in>())
            }
            SocketAddr::V6(addr) => {
                let mut sin_addr: CSockAddr = unsafe { std::mem::zeroed() };
                sin_addr.v6.sin6_family = AF_INET as u16;
                sin_addr.v6.sin6_addr.s6_addr = addr.ip().octets();
                sin_addr.v6.sin6_port = libc::htons(addr.port());

                (sin_addr, std::mem::size_of::<libc::sockaddr_in6>())
            }
        };

        let ret =
            unsafe { libc::connect(self.fd.as_raw_fd(), &addr as *const _ as *const _, len as _) };

        if ret == 0 {
            return Poll::Ready(Ok(()));
        }

        let err = std::io::Error::last_os_error();

        match err.raw_os_error().unwrap() {
            EINPROGRESS | EALREADY => {
                Reactor::get().register_waker(
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
