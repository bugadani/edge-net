#![allow(async_fn_in_trait)]

use std::io::{self, ErrorKind};
use std::net::{self, TcpStream, ToSocketAddrs, UdpSocket};
use std::os::fd::{AsFd, AsRawFd};

use async_io::Async;
use futures_lite::io::{AsyncReadExt, AsyncWriteExt};

use embedded_io_async::{ErrorType, Read, Write};

use embedded_nal_async::{
    AddrType, ConnectedUdp, Dns, IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6,
    TcpConnect, UdpStack, UnconnectedUdp,
};

use embedded_nal_async_xtra::{
    Multicast, RawSocket, RawStack, TcpAccept, TcpListen, TcpSplittableConnection,
};

pub struct StdTcpConnect(());

impl StdTcpConnect {
    pub const fn new() -> Self {
        Self(())
    }
}

impl TcpConnect for StdTcpConnect {
    type Error = io::Error;

    type Connection<'a> = StdTcpConnection where Self: 'a;

    async fn connect(&self, remote: SocketAddr) -> Result<Self::Connection<'_>, Self::Error> {
        let connection = Async::<TcpStream>::connect(to_std_addr(remote)).await?;

        Ok(StdTcpConnection(connection))
    }
}

pub struct StdTcpListen(());

impl StdTcpListen {
    pub const fn new() -> Self {
        Self(())
    }
}

impl TcpListen for StdTcpListen {
    type Error = io::Error;

    type Acceptor<'m>
    = StdTcpAccept where Self: 'm;

    async fn listen(&self, remote: SocketAddr) -> Result<Self::Acceptor<'_>, Self::Error> {
        Async::<net::TcpListener>::bind(to_std_addr(remote)).map(StdTcpAccept)
    }
}

pub struct StdTcpAccept(Async<net::TcpListener>);

impl TcpAccept for StdTcpAccept {
    type Error = io::Error;

    type Connection<'m> = StdTcpConnection;

    async fn accept(&self) -> Result<Self::Connection<'_>, Self::Error> {
        let connection = self.0.accept().await.map(|(socket, _)| socket)?;

        Ok(StdTcpConnection(connection))
    }
}

pub struct StdTcpConnection(Async<TcpStream>);

impl ErrorType for StdTcpConnection {
    type Error = io::Error;
}

impl Read for StdTcpConnection {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl Write for StdTcpConnection {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

impl TcpSplittableConnection for StdTcpConnection {
    type Error = io::Error;

    type Read<'a> = StdTcpConnectionRef<'a> where Self: 'a;

    type Write<'a> = StdTcpConnectionRef<'a> where Self: 'a;

    async fn split(&mut self) -> Result<(Self::Read<'_>, Self::Write<'_>), io::Error> {
        Ok((StdTcpConnectionRef(&self.0), StdTcpConnectionRef(&self.0)))
    }
}

pub struct StdTcpConnectionRef<'r>(&'r Async<TcpStream>);

impl<'r> ErrorType for StdTcpConnectionRef<'r> {
    type Error = io::Error;
}

impl<'r> Read for StdTcpConnectionRef<'r> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl<'r> Write for StdTcpConnectionRef<'r> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

pub struct StdUdpStack(());

impl StdUdpStack {
    pub const fn new() -> Self {
        Self(())
    }
}

impl UdpStack for StdUdpStack {
    type Error = io::Error;

    type Connected = StdUdpSocket;

    type UniquelyBound = StdUdpSocket;

    type MultiplyBound = StdUdpSocket;

    async fn connect_from(
        &self,
        local: SocketAddr,
        remote: SocketAddr,
    ) -> Result<(SocketAddr, Self::Connected), Self::Error> {
        let socket = Async::<UdpSocket>::bind(to_std_addr(local))?;

        socket.as_ref().connect(to_std_addr(remote))?;

        Ok((
            to_nal_addr(socket.as_ref().local_addr()?),
            StdUdpSocket(socket),
        ))
    }

    async fn bind_single(
        &self,
        local: SocketAddr,
    ) -> Result<(SocketAddr, Self::UniquelyBound), Self::Error> {
        let socket = Async::<UdpSocket>::bind(to_std_addr(local))?;

        socket.as_ref().set_broadcast(true)?;

        Ok((
            to_nal_addr(socket.as_ref().local_addr()?),
            StdUdpSocket(socket),
        ))
    }

    async fn bind_multiple(&self, _local: SocketAddr) -> Result<Self::MultiplyBound, Self::Error> {
        unimplemented!() // TODO
    }
}

pub struct StdUdpSocket(Async<UdpSocket>);

impl ConnectedUdp for StdUdpSocket {
    type Error = io::Error;

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let mut offset = 0;

        loop {
            offset += self.0.send(&data[offset..]).await?;

            if offset == data.len() {
                break;
            }
        }

        Ok(())
    }

    async fn receive_into(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.recv(buffer).await
    }
}

impl UnconnectedUdp for StdUdpSocket {
    type Error = io::Error;

    async fn send(
        &mut self,
        local: SocketAddr,
        remote: SocketAddr,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        assert!(local == to_nal_addr(self.0.as_ref().local_addr()?));

        let mut offset = 0;

        loop {
            offset += self.0.send_to(data, to_std_addr(remote)).await?;

            if offset == data.len() {
                break;
            }
        }

        Ok(())
    }

    async fn receive_into(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<(usize, SocketAddr, SocketAddr), Self::Error> {
        let (len, addr) = self.0.recv_from(buffer).await?;

        Ok((
            len,
            to_nal_addr(self.0.as_ref().local_addr()?),
            to_nal_addr(addr),
        ))
    }
}

impl Multicast for StdUdpSocket {
    type Error = io::Error;

    async fn join(&mut self, multicast_addr: IpAddr) -> Result<(), Self::Error> {
        match multicast_addr {
            IpAddr::V4(addr) => self
                .0
                .as_ref()
                .join_multicast_v4(&addr.octets().into(), &std::net::Ipv4Addr::UNSPECIFIED)?,
            IpAddr::V6(addr) => self
                .0
                .as_ref()
                .join_multicast_v6(&addr.octets().into(), 0)?,
        }

        Ok(())
    }

    async fn leave(&mut self, multicast_addr: IpAddr) -> Result<(), Self::Error> {
        match multicast_addr {
            IpAddr::V4(addr) => self
                .0
                .as_ref()
                .leave_multicast_v4(&addr.octets().into(), &std::net::Ipv4Addr::UNSPECIFIED)?,
            IpAddr::V6(addr) => self
                .0
                .as_ref()
                .leave_multicast_v6(&addr.octets().into(), 0)?,
        }

        Ok(())
    }
}

pub struct StdDns(());

impl StdDns {
    pub const fn new() -> Self {
        Self(())
    }
}

impl Dns for StdDns {
    type Error = io::Error;

    async fn get_host_by_name(
        &self,
        host: &str,
        addr_type: AddrType,
    ) -> Result<IpAddr, Self::Error> {
        let host = host.to_string();

        dns_lookup_host(&host, addr_type)
    }

    async fn get_host_by_address(
        &self,
        _addr: IpAddr,
        _result: &mut [u8],
    ) -> Result<usize, Self::Error> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn dns_lookup_host(host: &str, addr_type: AddrType) -> Result<IpAddr, io::Error> {
    (host, 0_u16)
        .to_socket_addrs()?
        .find(|addr| match addr_type {
            AddrType::IPv4 => matches!(addr, std::net::SocketAddr::V4(_)),
            AddrType::IPv6 => matches!(addr, std::net::SocketAddr::V6(_)),
            AddrType::Either => true,
        })
        .map(|addr| match addr {
            std::net::SocketAddr::V4(v4) => v4.ip().octets().into(),
            std::net::SocketAddr::V6(v6) => v6.ip().octets().into(),
        })
        .ok_or_else(|| io::ErrorKind::AddrNotAvailable.into())
}

#[cfg(unix)]
pub struct StdRawSocket(Async<std::net::UdpSocket>, u32);

#[cfg(unix)]
impl RawSocket for StdRawSocket {
    type Error = io::Error;

    async fn send(&mut self, mac: Option<&[u8; 6]>, data: &[u8]) -> Result<(), Self::Error> {
        let mut sockaddr = libc::sockaddr_ll {
            sll_family: libc::AF_PACKET as _,
            sll_protocol: (libc::ETH_P_IP as u16).to_be() as _,
            sll_ifindex: self.1 as _,
            sll_hatype: 0,
            sll_pkttype: 0,
            sll_halen: 0,
            sll_addr: Default::default(),
        };

        if let Some(mac) = mac {
            sockaddr.sll_halen = mac.len() as _;
            sockaddr.sll_addr[..mac.len()].copy_from_slice(mac);
        }

        let len = self
            .0
            .write_with(|io| {
                let len = core::cmp::min(data.len(), u16::MAX as usize);

                let ret = cvti(unsafe {
                    libc::sendto(
                        io.as_fd().as_raw_fd(),
                        data.as_ptr() as *const _,
                        len,
                        libc::MSG_NOSIGNAL,
                        &sockaddr as *const _ as *const _,
                        core::mem::size_of::<libc::sockaddr_ll>() as _,
                    )
                })?;
                Ok(ret as usize)
            })
            .await?;

        assert_eq!(len, data.len());

        Ok(())
    }

    async fn receive_into(&mut self, buffer: &mut [u8]) -> Result<(usize, [u8; 6]), Self::Error> {
        self.0
            .read_with(|io| {
                let mut storage: libc::sockaddr_storage = unsafe { core::mem::zeroed() };
                let mut addrlen = core::mem::size_of_val(&storage) as libc::socklen_t;

                let ret = cvti(unsafe {
                    libc::recvfrom(
                        io.as_fd().as_raw_fd(),
                        buffer.as_mut_ptr() as *mut _,
                        buffer.len(),
                        0,
                        &mut storage as *mut _ as *mut _,
                        &mut addrlen,
                    )
                })?;

                let sockaddr = as_sockaddr_ll(&storage, addrlen as usize)?;

                let mut mac = [0; 6];
                mac.copy_from_slice(&sockaddr.sll_addr[..6]);

                Ok((ret as usize, mac))
            })
            .await
    }
}

#[cfg(unix)]
pub struct StdRawStack(u32);

#[cfg(unix)]
impl StdRawStack {
    pub const fn new(interface: u32) -> Self {
        Self(interface)
    }
}

#[cfg(unix)]
impl RawStack for StdRawStack {
    type Error = io::Error;

    type Socket = StdRawSocket;

    async fn bind(&self) -> Result<Self::Socket, Self::Error> {
        let socket = cvt(unsafe {
            libc::socket(
                libc::PF_PACKET,
                libc::SOCK_DGRAM,
                (libc::ETH_P_IP as u16).to_be() as _,
            )
        })?;

        let sockaddr = libc::sockaddr_ll {
            sll_family: libc::AF_PACKET as _,
            sll_protocol: (libc::ETH_P_IP as u16).to_be() as _,
            sll_ifindex: self.0 as _,
            sll_hatype: 0,
            sll_pkttype: 0,
            sll_halen: 0,
            sll_addr: Default::default(),
        };

        cvt(unsafe {
            libc::bind(
                socket,
                &sockaddr as *const _ as *const _,
                core::mem::size_of::<libc::sockaddr_ll>() as _,
            )
        })?;

        // TODO
        // cvt(unsafe {
        //     libc::setsockopt(socket, libc::SOL_PACKET, libc::PACKET_AUXDATA, &1_u32 as *const _ as *const _, 4)
        // })?;

        let socket = {
            use std::os::fd::FromRawFd;

            unsafe { std::net::UdpSocket::from_raw_fd(socket) }
        };

        socket.set_broadcast(true)?;

        Ok(StdRawSocket(Async::new(socket)?, self.0 as _))
    }
}

fn as_sockaddr_ll(storage: &libc::sockaddr_storage, len: usize) -> io::Result<&libc::sockaddr_ll> {
    match storage.ss_family as core::ffi::c_int {
        libc::AF_PACKET => {
            assert!(len >= core::mem::size_of::<libc::sockaddr_ll>());
            Ok(unsafe { (storage as *const _ as *const libc::sockaddr_ll).as_ref() }.unwrap())
        }
        _ => Err(io::Error::new(ErrorKind::InvalidInput, "invalid argument")),
    }
}

pub fn to_std_addr(addr: SocketAddr) -> std::net::SocketAddr {
    match addr {
        SocketAddr::V4(addr) => net::SocketAddr::V4(net::SocketAddrV4::new(
            addr.ip().octets().into(),
            addr.port(),
        )),
        SocketAddr::V6(addr) => net::SocketAddr::V6(net::SocketAddrV6::new(
            addr.ip().octets().into(),
            addr.port(),
            addr.flowinfo(),
            addr.scope_id(),
        )),
    }
}

pub fn to_nal_addr(addr: std::net::SocketAddr) -> SocketAddr {
    match addr {
        net::SocketAddr::V4(addr) => {
            SocketAddr::V4(SocketAddrV4::new(addr.ip().octets().into(), addr.port()))
        }
        net::SocketAddr::V6(addr) => SocketAddr::V6(SocketAddrV6::new(
            addr.ip().octets().into(),
            addr.port(),
            addr.flowinfo(),
            addr.scope_id(),
        )),
    }
}

pub fn to_std_ipv4_addr(addr: Ipv4Addr) -> std::net::Ipv4Addr {
    addr.octets().into()
}

pub fn to_nal_ipv4_addr(addr: std::net::Ipv4Addr) -> Ipv4Addr {
    addr.octets().into()
}

fn cvt<T>(res: T) -> io::Result<T>
where
    T: Into<i64> + Copy,
{
    let ires: i64 = res.into();

    if ires == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(res)
    }
}

fn cvti<T>(res: T) -> io::Result<T>
where
    T: Into<isize> + Copy,
{
    let ires: isize = res.into();

    if ires == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(res)
    }
}
