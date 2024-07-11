use crate::{locking::MutexGuard, Error};
use rtsc::data_policy::DataDeliveryPolicy;
use std::{
    io::{Read, Write},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use crate::Result;

pub mod serial; // Serial communications
pub mod tcp; // TCP communications

/// A versatile (TCP/serial) client
#[derive(Clone)]
pub struct Client(Arc<dyn Communicator + Send + Sync>);

impl Client {
    /// Lock the client for exclusive access
    pub fn lock(&self) -> MutexGuard<()> {
        self.0.lock()
    }
    /// Connect the client. Does not need to be called for request/response protocols as the client
    /// is automatically connected when the first request is made.
    pub fn connect(&self) -> Result<()> {
        self.0.connect()
    }
    /// Reconnect the client in case of read/write problems
    pub fn reconnect(&self) {
        self.0.reconnect();
    }
    /// Write data to the client
    pub fn write(&self, buf: &[u8]) -> Result<()> {
        self.0.write(buf).map_err(Into::into)
    }
    /// Read data from the client
    pub fn read_exact(&self, buf: &mut [u8]) -> Result<()> {
        self.0.read_exact(buf)
    }
    /// Get the protocol of the client
    pub fn protocol(&self) -> Protocol {
        self.0.protocol()
    }
    /// Get local IP address (for TCP/IP)
    pub fn local_ip_addr(&self) -> Result<Option<SocketAddr>> {
        self.0.local_ip_addr()
    }
    /// Get the current session id
    pub fn session_id(&self) -> usize {
        self.0.session_id()
    }
    /// lock the current session (disable reconnects)
    pub fn lock_session(&self) -> Result<SessionGuard> {
        let session_id = self.0.lock_session()?;
        Ok(SessionGuard {
            client: self.clone(),
            session_id,
        })
    }
}

impl Read for Client {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.read_exact(buf) {
            Ok(()) => Ok(buf.len()),
            Err(Error::IO(e)) => Err(e),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        }
    }
}

impl Write for Client {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.0.write(buf) {
            Ok(()) => Ok(buf.len()),
            Err(Error::IO(e)) => Err(e),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct SessionGuard {
    client: Client,
    session_id: usize,
}

impl SessionGuard {
    pub fn session_id(&self) -> usize {
        self.session_id
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.client.0.unlock_session();
    }
}

pub enum Protocol {
    Tcp,
    Serial,
}

pub trait Stream: Read + Write + Send {}

trait Communicator {
    fn lock(&self) -> MutexGuard<()>;
    fn connect(&self) -> Result<()>;
    fn reconnect(&self);
    fn write(&self, buf: &[u8]) -> Result<()>;
    fn read_exact(&self, buf: &mut [u8]) -> Result<()>;
    fn protocol(&self) -> Protocol;
    fn session_id(&self) -> usize;
    fn local_ip_addr(&self) -> Result<Option<SocketAddr>> {
        Ok(None)
    }
    fn lock_session(&self) -> Result<usize>;
    fn unlock_session(&self);
}

#[allow(clippy::module_name_repetitions)]
pub struct CommReader {
    reader: Option<Box<dyn Read + Send + 'static>>,
}

impl CommReader {
    pub fn take(&mut self) -> Option<Box<dyn Read + Send + 'static>> {
        self.reader.take()
    }
}

impl DataDeliveryPolicy for CommReader {}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct Timeouts {
    pub connect: Duration,
    pub read: Duration,
    pub write: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self::new(DEFAULT_TIMEOUT)
    }
}

impl Timeouts {
    pub fn new(default: Duration) -> Self {
        Self {
            connect: default,
            read: default,
            write: default,
        }
    }
    pub fn none() -> Self {
        Self {
            connect: Duration::from_secs(0),
            read: Duration::from_secs(0),
            write: Duration::from_secs(0),
        }
    }
}

pub trait ConnectionHandler {
    /// called right after the connection is established
    fn on_connect(
        &self,
        stream: &mut dyn Stream,
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Connection Options
pub struct ConnectionOptions {
    with_reader: bool,
    connection_handler: Option<Box<dyn ConnectionHandler + Send + Sync>>,
    timeouts: Timeouts,
}

impl ConnectionOptions {
    /// timeout = the default timeout
    pub fn new(timeout: Duration) -> Self {
        Self {
            with_reader: false,
            connection_handler: None,
            timeouts: Timeouts {
                connect: timeout,
                read: timeout,
                write: timeout,
            },
        }
    }
    /// Enable the reader channel. The reader channel allows the client to receive a clone of the
    /// stream reader when the connection is established. This is useful for implementing custom
    /// protocols that require reading from the stream.
    pub fn with_reader(mut self) -> Self {
        self.with_reader = true;
        self
    }
    /// Set the connection handler. The connection handler is used to implement custom protocols
    /// that require additional setup/handling. Replaces "chat" function.
    pub fn connection_handler<T>(mut self, connection_handler: T) -> Self
    where
        T: ConnectionHandler + Send + Sync + 'static,
    {
        self.connection_handler = Some(Box::new(connection_handler));
        self
    }
    /// Set timeouts
    pub fn timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
    /// Set the connect timeout
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.timeouts.connect = timeout;
        self
    }
    /// Set the read timeout
    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.timeouts.read = timeout;
        self
    }
    /// Set the write timeout
    pub fn write_timeout(mut self, timeout: Duration) -> Self {
        self.timeouts.write = timeout;
        self
    }
}
