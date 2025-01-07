//!
//! Can be used to communicate between processes on different machines or with various 3rd party
//! devices and software, such as Matlab, LabView, etc.
//!
//! [Raw UDP example](https://github.com/roboplc/roboplc/blob/main/examples/raw-udp.rs)
use binrw::{BinRead, BinWrite};
use std::{
    io::Cursor,
    marker::PhantomData,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
};

use crate::{Error, Result};

/// Raw UDP receiver
pub struct UdpReceiver<T>
where
    T: for<'a> BinRead<Args<'a> = ()>,
{
    server: UdpSocket,
    buffer: Vec<u8>,
    _phantom: PhantomData<T>,
}

impl<T> UdpReceiver<T>
where
    T: for<'a> BinRead<Args<'a> = ()>,
{
    /// Binds to the specified address and creates a new receiver
    pub fn bind<A: ToSocketAddrs>(addr: A, buf_size: usize) -> Result<Self> {
        let server = UdpSocket::bind(addr)?;
        Ok(Self {
            server,
            buffer: vec![0; buf_size],
            _phantom: PhantomData,
        })
    }
}

impl<T> Iterator for UdpReceiver<T>
where
    T: for<'a> BinRead<Args<'a> = ()>,
{
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.server.recv(&mut self.buffer) {
            Ok(size) => {
                let mut cursor = Cursor::new(&self.buffer[..size]);
                Some(T::read_le(&mut cursor).map_err(Into::into))
            }
            Err(e) => Some(Err(e.into())),
        }
    }
}

/// Raw UDP sender
pub struct UdpSender<T>
where
    T: for<'a> BinWrite<Args<'a> = ()>,
{
    socket: UdpSocket,
    target: SocketAddr,
    data_buf: Vec<u8>,
    // keep the generic `T` global (including traits) as each instance is dedicated to send a
    // specific type only
    _phantom: PhantomData<T>,
}

impl<T> UdpSender<T>
where
    T: for<'a> BinWrite<Args<'a> = ()>,
{
    /// Connects to the specified address and creates a new sender
    pub fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        let target = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| Error::InvalidData("no target address provided".to_string()))?;
        Ok(Self {
            socket,
            target,
            data_buf: <_>::default(),
            _phantom: PhantomData,
        })
    }

    /// Sends a value to the target address
    pub fn send(&mut self, value: &T) -> Result<()> {
        let mut buf = Cursor::new(&mut self.data_buf);
        value.write_le(&mut buf)?;
        self.socket.send_to(&self.data_buf, self.target)?;
        Ok(())
    }
}
