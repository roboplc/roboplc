use parking_lot::MutexGuard;
use std::sync::Arc;

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
    pub fn write(&self, buf: &[u8]) -> Result<(), std::io::Error> {
        self.0.write(buf)
    }
    /// Read data from the client
    pub fn read_exact(&self, buf: &mut [u8]) -> Result<(), std::io::Error> {
        self.0.read_exact(buf)
    }
    /// Get the protocol of the client
    pub fn protocol(&self) -> Protocol {
        self.0.protocol()
    }
}

pub enum Protocol {
    Tcp,
    Serial,
}

trait Communicator {
    fn lock(&self) -> MutexGuard<()>;
    fn reconnect(&self);
    fn write(&self, buf: &[u8]) -> Result<(), std::io::Error>;
    fn read_exact(&self, buf: &mut [u8]) -> Result<(), std::io::Error>;
    fn protocol(&self) -> Protocol;
}
