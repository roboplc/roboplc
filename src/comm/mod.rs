use parking_lot::MutexGuard;
use std::sync::Arc;

pub mod serial;
pub mod tcp;

#[derive(Clone)]
pub struct Client(Arc<dyn Communicator + Send + Sync>);

impl Client {
    pub fn lock(&self) -> MutexGuard<()> {
        self.0.lock()
    }
    pub fn reconnect(&self) {
        self.0.reconnect();
    }
    pub fn write(&self, buf: &[u8]) -> Result<(), std::io::Error> {
        self.0.write(buf)
    }
    pub fn read_exact(&self, buf: &mut [u8]) -> Result<(), std::io::Error> {
        self.0.read_exact(buf)
    }
    pub fn protocol(&self) -> Protocol {
        self.0.protocol()
    }
}

pub enum Protocol {
    Tcp,
    Rtu,
}

trait Communicator {
    fn lock(&self) -> MutexGuard<()>;
    fn reconnect(&self);
    fn write(&self, buf: &[u8]) -> Result<(), std::io::Error>;
    fn read_exact(&self, buf: &mut [u8]) -> Result<(), std::io::Error>;
    fn protocol(&self) -> Protocol;
}
