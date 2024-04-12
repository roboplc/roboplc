use parking_lot::MutexGuard;
use std::{
    io::{Read, Write},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use crate::{DataDeliveryPolicy, Result};

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
    pub fn local_ip_addr(&self) -> Result<Option<SocketAddr>> {
        self.0.local_ip_addr()
    }
    pub fn session_id(&self) -> usize {
        self.0.session_id()
    }
}

pub enum Protocol {
    Tcp,
    Serial,
}

pub trait Stream: Read + Write + Send {}

trait Communicator {
    fn lock(&self) -> MutexGuard<()>;
    fn reconnect(&self);
    fn write(&self, buf: &[u8]) -> Result<()>;
    fn read_exact(&self, buf: &mut [u8]) -> Result<()>;
    fn protocol(&self) -> Protocol;
    fn session_id(&self) -> usize;
    fn local_ip_addr(&self) -> Result<Option<SocketAddr>> {
        Ok(None)
    }
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

#[derive(Default, Clone)]
pub struct Timeouts {
    pub connect: Duration,
    pub read: Duration,
    pub write: Duration,
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

pub type ChatFn = dyn Fn(&mut dyn Stream) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>
    + Send
    + Sync;

/// Connection Options
pub struct ConnectionOptions {
    with_reader: bool,
    chat: Option<Box<ChatFn>>,
    timeouts: Timeouts,
}

impl ConnectionOptions {
    /// timeout = the default timeout
    pub fn new(timeout: Duration) -> Self {
        Self {
            with_reader: false,
            chat: None,
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
    /// Set the chat function. The chat function is called after the connection is established. The
    /// chat function can be used to implement custom protocols that require additional setup.
    pub fn chat<F>(mut self, chat: F) -> Self
    where
        F: Fn(&mut dyn Stream) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>
            + Send
            + Sync
            + 'static,
    {
        self.chat = Some(Box::new(chat));
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
