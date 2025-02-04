use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

pub union CSockAddrs {
    v4: libc::sockaddr_in,
    v6: libc::sockaddr_in6,
}

pub struct CSockAddr {
    pub addr: CSockAddrs,
    pub len: usize,
}

impl From<SocketAddr> for CSockAddr {
    fn from(value: SocketAddr) -> Self {
        match value {
            SocketAddr::V4(addr) => {
                let mut sin_addr: CSockAddrs = unsafe { std::mem::zeroed() };
                sin_addr.v4.sin_family = libc::AF_INET as u16;
                sin_addr.v4.sin_addr.s_addr = libc::htonl(addr.ip().to_bits());
                sin_addr.v4.sin_port = libc::htons(addr.port());

                CSockAddr {
                    addr: sin_addr,
                    len: std::mem::size_of::<libc::sockaddr_in>(),
                }
            }
            SocketAddr::V6(addr) => {
                let mut sin_addr: CSockAddrs = unsafe { std::mem::zeroed() };
                sin_addr.v6.sin6_family = libc::AF_INET6 as u16;
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

impl TryFrom<&CSockAddr> for SocketAddr {
    type Error = std::io::Error;

    fn try_from(value: &CSockAddr) -> Result<Self, Self::Error> {
        const V4_LEN: usize = std::mem::size_of::<libc::sockaddr_in>();
        const V6_LEN: usize = std::mem::size_of::<libc::sockaddr_in6>();

        match value.len {
            V4_LEN => unsafe {
                Ok(SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::from_bits(libc::ntohl(value.addr.v4.sin_addr.s_addr)),
                    libc::ntohs(value.addr.v4.sin_port),
                )))
            },
            V6_LEN => unsafe {
                Ok(SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from_bits(u128::from_be_bytes(value.addr.v6.sin6_addr.s6_addr)),
                    libc::ntohs(value.addr.v6.sin6_port),
                    value.addr.v6.sin6_flowinfo,
                    value.addr.v6.sin6_scope_id,
                )))
            },
            _ => Err(std::io::ErrorKind::InvalidData.into()),
        }
    }
}

impl CSockAddr {
    pub fn as_ptr(&self) -> *const libc::sockaddr {
        &self.addr as *const _ as *const _
    }
}
