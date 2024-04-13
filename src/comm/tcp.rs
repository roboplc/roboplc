use crate::pchannel;
use crate::{Error, Result};

use super::{
    ChatFn, Client, CommReader, Communicator, ConnectionOptions, Protocol, Stream, Timeouts,
};
use core::fmt;
use parking_lot::{Mutex, MutexGuard};
use std::io::{Read, Write};
use std::net::{self, TcpStream};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const READER_CHANNEL_CAPACITY: usize = 1024;

/// Create a new TCP client. The client will attempt to connect to the given address at the time of
/// the first request. The client will automatically reconnect if the connection is lost.
pub fn connect<A: ToSocketAddrs + fmt::Debug>(addr: A, timeout: Duration) -> Result<Client> {
    Ok(Client(
        Tcp::create(addr, ConnectionOptions::new(timeout))?.0,
    ))
}

/// Create a new TCP client with options. The client will attempt to connect to the given address
/// at the time of the first request. The client will automatically reconnect if the connection is
/// lost.
pub fn connect_with_options<A: ToSocketAddrs + fmt::Debug>(
    addr: A,
    options: ConnectionOptions,
) -> Result<(Client, Option<pchannel::Receiver<CommReader>>)> {
    let (tcp, maybe_rx) = Tcp::create(addr, options)?;
    Ok((Client(tcp), maybe_rx))
}

impl Stream for TcpStream {}

#[allow(clippy::module_name_repetitions)]
pub struct Tcp {
    addr: SocketAddr,
    stream: Mutex<Option<TcpStream>>,
    timeouts: Timeouts,
    busy: Mutex<()>,
    session_id: AtomicUsize,
    reader_tx: Option<pchannel::Sender<CommReader>>,
    chat: Option<Box<ChatFn>>,
}

#[allow(clippy::module_name_repetitions)]
pub type TcpClient = Arc<Tcp>;

macro_rules! handle_tcp_stream_error {
    ($stream: expr, $err: expr, $any: expr) => {{
        if $any || $err.kind() == std::io::ErrorKind::TimedOut {
            $stream.take().map(|s| s.shutdown(net::Shutdown::Both));
        }
        $err.into()
    }};
}

impl Communicator for Tcp {
    fn lock(&self) -> MutexGuard<()> {
        self.busy.lock()
    }
    fn session_id(&self) -> usize {
        self.session_id.load(Ordering::Acquire)
    }
    fn reconnect(&self) {
        self.stream
            .lock()
            .take()
            .map(|s| s.shutdown(net::Shutdown::Both));
    }
    fn write(&self, buf: &[u8]) -> Result<()> {
        let mut stream = self.get_stream()?;
        stream
            .as_mut()
            .unwrap()
            .write_all(buf)
            .map_err(|e| handle_tcp_stream_error!(stream, e, true))
    }
    fn read_exact(&self, buf: &mut [u8]) -> Result<()> {
        let mut stream = self.get_stream()?;
        stream
            .as_mut()
            .unwrap()
            .read_exact(buf)
            .map_err(|e| handle_tcp_stream_error!(stream, e, false))
    }
    fn local_ip_addr(&self) -> Result<Option<SocketAddr>> {
        let mut stream = self.get_stream()?;
        stream
            .as_mut()
            .unwrap()
            .local_addr()
            .map(Some)
            .map_err(|e| handle_tcp_stream_error!(stream, e, false))
    }
    fn protocol(&self) -> Protocol {
        Protocol::Tcp
    }
}

impl Tcp {
    fn create<A: ToSocketAddrs + fmt::Debug>(
        addr: A,
        options: ConnectionOptions,
    ) -> Result<(TcpClient, Option<pchannel::Receiver<CommReader>>)> {
        let (tx, rx) = if options.with_reader {
            let (tx, rx) = pchannel::bounded(READER_CHANNEL_CAPACITY);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let client = Self {
            addr: addr
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| Error::invalid_data(format!("Invalid address: {:?}", addr)))?,
            stream: <_>::default(),
            busy: <_>::default(),
            timeouts: options.timeouts,
            session_id: <_>::default(),
            reader_tx: tx,
            chat: options.chat,
        };
        Ok((client.into(), rx))
    }
    fn get_stream(&self) -> Result<MutexGuard<Option<TcpStream>>> {
        let mut lock = self.stream.lock();
        if lock.as_mut().is_none() {
            let zero_to = Duration::from_secs(0);
            let mut stream = if self.timeouts.connect > zero_to {
                TcpStream::connect_timeout(&self.addr, self.timeouts.connect)?
            } else {
                TcpStream::connect(self.addr)?
            };
            if self.timeouts.read > zero_to {
                stream.set_read_timeout(Some(self.timeouts.read))?;
            }
            if self.timeouts.write > zero_to {
                stream.set_write_timeout(Some(self.timeouts.write))?;
            }
            stream.set_nodelay(true)?;
            if let Some(ref chat) = self.chat {
                chat(&mut stream).map_err(Error::io)?;
            }
            self.session_id.fetch_add(1, Ordering::Release);
            if let Some(ref tx) = self.reader_tx {
                tx.send(CommReader {
                    reader: Some(Box::new(stream.try_clone()?)),
                })?;
            }
            lock.replace(stream);
        }
        Ok(lock)
    }
}

impl Drop for Tcp {
    fn drop(&mut self) {
        self.stream
            .lock()
            .take()
            .map(|s| s.shutdown(net::Shutdown::Both));
    }
}
