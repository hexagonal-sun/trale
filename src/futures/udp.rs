use std::future::Future;
use std::io;
use std::io::Result;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::net::UdpSocket as StdUdpSocket;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use crate::reactor::Reactor;
use crate::reactor::WakeupKind;

pub struct UdpSocket {
    inner: Arc<StdUdpSocket>,
}

impl UdpSocket {
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let sock = StdUdpSocket::bind(addr)?;

        sock.set_nonblocking(true)?;

        Ok(Self {
            inner: Arc::new(sock),
        })
    }

    pub fn recv_from<'a>(&'a mut self, buf: &'a mut [u8]) -> RecvFrom {
        RecvFrom {
            sock: self.inner.clone(),
            buf,
        }
    }

    pub fn send_to<'a, A: ToSocketAddrs>(&'a self, buf: &'a [u8], target: A) -> SendTo<A> {
        SendTo {
            sock: self.inner.clone(),
            dst: target,
            buf,
        }
    }
}

pub struct RecvFrom<'a> {
    sock: Arc<StdUdpSocket>,
    buf: &'a mut [u8],
}

impl Future for RecvFrom<'_> {
    type Output = Result<(usize, SocketAddr)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.sock.clone().recv_from(self.buf) {
            Ok(ret) => Poll::Ready(Ok(ret)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                Reactor::get().register_waker(
                    self.sock.clone(),
                    cx.waker().clone(),
                    WakeupKind::Readable,
                );

                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

pub struct SendTo<'a, A: ToSocketAddrs> {
    sock: Arc<StdUdpSocket>,
    dst: A,
    buf: &'a [u8],
}

impl<A: ToSocketAddrs> Future for SendTo<'_, A> {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.sock.clone().send_to(self.buf, &self.dst) {
            Ok(ret) => Poll::Ready(Ok(ret)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                Reactor::get().register_waker(
                    self.sock.clone(),
                    cx.waker().clone(),
                    WakeupKind::Writable,
                );

                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}
