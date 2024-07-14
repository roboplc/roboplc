use crate::{Error, Result};

use super::Client;
use super::Communicator;
use super::Protocol;
use crate::locking::{Mutex, MutexGuard};
use serial::prelude::*;
use serial::SystemPort;
use std::io;
use std::io::{Read, Write};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::trace;

/// Create a new serial client. The client will attempt to connect to the given address at the time
/// of the first request. The client will automatically reconnect if the connection is lost.
///
/// Path syntax: `port_dev:baud_rate:char_size:parity:stop_bits`, e.g. `/dev/ttyS0:9600:8:N:1`
pub fn connect(path: &str, timeout: Duration, frame_delay: Duration) -> Result<Client> {
    Ok(Client(Serial::create(path, timeout, frame_delay)?))
}

/// Serial port parameters
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Parameters {
    /// Serial port device path
    pub port_dev: String,
    /// Baud rate
    pub baud_rate: serial::BaudRate,
    /// Character size
    pub char_size: serial::CharSize,
    /// Parity
    pub parity: serial::Parity,
    /// Stop bits
    pub stop_bits: serial::StopBits,
}

impl FromStr for Parameters {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        parse_path(s)
    }
}

fn parse_path(path: &str) -> Result<Parameters> {
    let mut sp = path.split(':');
    let port_dev = sp.next().unwrap();
    let s_baud_rate = sp
        .next()
        .ok_or_else(|| Error::invalid_data(format!("serial baud rate not specified: {}", path)))?;
    let s_char_size = sp
        .next()
        .ok_or_else(|| Error::invalid_data(format!("serial char size not specified: {}", path)))?;
    let s_parity = sp
        .next()
        .ok_or_else(|| Error::invalid_data(format!("serial parity not specified: {}", path)))?;
    let s_stop_bits = sp
        .next()
        .ok_or_else(|| Error::invalid_data(format!("serial stopbits not specified: {}", path)))?;
    let baud_rate = match s_baud_rate {
        "110" => serial::Baud110,
        "300" => serial::Baud300,
        "600" => serial::Baud600,
        "1200" => serial::Baud1200,
        "2400" => serial::Baud2400,
        "4800" => serial::Baud4800,
        "9600" => serial::Baud9600,
        "19200" => serial::Baud19200,
        "38400" => serial::Baud38400,
        "57600" => serial::Baud57600,
        "115200" => serial::Baud115200,
        v => {
            return Err(Error::invalid_data(format!(
                "specified serial baud rate not supported: {}",
                v
            )))
        }
    };
    let char_size = match s_char_size {
        "5" => serial::Bits5,
        "6" => serial::Bits6,
        "7" => serial::Bits7,
        "8" => serial::Bits8,
        v => {
            return Err(Error::invalid_data(format!(
                "specified serial char size not supported: {}",
                v
            )))
        }
    };
    let parity = match s_parity {
        "N" => serial::ParityNone,
        "E" => serial::ParityEven,
        "O" => serial::ParityOdd,
        v => {
            return Err(Error::invalid_data(format!(
                "specified serial parity not supported: {}",
                v
            )))
        }
    };
    let stop_bits = match s_stop_bits {
        "1" => serial::Stop1,
        "2" => serial::Stop2,
        v => unimplemented!("specified serial stop bits not supported: {}", v),
    };
    Ok(Parameters {
        port_dev: port_dev.to_owned(),
        baud_rate,
        char_size,
        parity,
        stop_bits,
    })
}

/// Open a serial port
pub fn open(params: &Parameters, timeout: Duration) -> Result<SystemPort> {
    let mut port = serial::open(&params.port_dev).map_err(Error::io)?;
    port.reconfigure(&|settings| {
        settings.set_baud_rate(params.baud_rate)?;
        settings.set_char_size(params.char_size);
        settings.set_parity(params.parity);
        settings.set_stop_bits(params.stop_bits);
        settings.set_flow_control(serial::FlowNone);
        Ok(())
    })
    .map_err(Error::io)?;
    if timeout > Duration::from_secs(0) {
        port.set_timeout(timeout).map_err(Error::io)?;
    }
    Ok(port)
}

/// Serial port client
#[allow(clippy::module_name_repetitions)]
pub struct Serial {
    port: Mutex<SPort>,
    timeout: Duration,
    frame_delay: Duration,
    busy: Mutex<()>,
    params: Parameters,
    session_id: AtomicUsize,
    allow_reconnect: AtomicBool,
}

#[derive(Default)]
struct SPort {
    system_port: Option<SystemPort>,
    last_frame: Option<Instant>,
}

/// Serial port client type
#[allow(clippy::module_name_repetitions)]
pub type SerialClient = Arc<Serial>;

impl Communicator for Serial {
    fn lock(&self) -> MutexGuard<()> {
        self.busy.lock()
    }
    fn session_id(&self) -> usize {
        self.session_id.load(Ordering::Acquire)
    }
    fn connect(&self) -> Result<()> {
        self.get_port().map(|_| ())
    }
    fn reconnect(&self) {
        let mut port = self.port.lock();
        port.system_port.take();
        port.last_frame.take();
    }
    fn write(&self, buf: &[u8]) -> Result<()> {
        let mut port = self
            .get_port()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if let Some(last_frame) = port.last_frame {
            let el = last_frame.elapsed();
            if el < self.frame_delay {
                std::thread::sleep(self.frame_delay - el);
            }
        }
        let result = port
            .system_port
            .as_mut()
            .unwrap()
            .write_all(buf)
            .map_err(|e| {
                self.reconnect();
                e
            });
        if result.is_ok() {
            port.last_frame.replace(Instant::now());
        }
        result.map_err(Into::into)
    }
    fn read_exact(&self, buf: &mut [u8]) -> Result<()> {
        let mut port = self
            .get_port()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        port.system_port
            .as_mut()
            .unwrap()
            .read_exact(buf)
            .map_err(|e| {
                self.reconnect();
                e
            })
            .map_err(Into::into)
    }
    fn protocol(&self) -> Protocol {
        Protocol::Serial
    }

    fn lock_session(&self) -> Result<usize> {
        let _lock = self.lock();
        let _s = self.get_port()?;
        self.allow_reconnect.store(false, Ordering::Release);
        Ok(self.session_id())
    }

    fn unlock_session(&self) {
        self.allow_reconnect.store(true, Ordering::Release);
    }
}

impl Serial {
    /// Create a new serial client
    pub fn create(path: &str, timeout: Duration, frame_delay: Duration) -> Result<Arc<Self>> {
        let params = parse_path(path)?;
        Ok(Self {
            port: <_>::default(),
            timeout,
            frame_delay,
            busy: <_>::default(),
            params,
            session_id: <_>::default(),
            allow_reconnect: AtomicBool::new(true),
        }
        .into())
    }
    fn get_port(&self) -> Result<MutexGuard<SPort>> {
        let mut lock = self.port.lock();
        if lock.system_port.as_mut().is_none() {
            if !self.allow_reconnect.load(Ordering::Acquire) {
                return Err(Error::io("not connected but reconnects not allowed"));
            }
            trace!(dev=%self.params.port_dev, "creating new serial connection");
            let port = open(&self.params, self.timeout)?;
            lock.system_port.replace(port);
            lock.last_frame.take();
            self.session_id.fetch_add(1, Ordering::Release);
            trace!(dev=%self.params.port_dev, session_id=self.session_id(), "serial connection started");
        }
        Ok(lock)
    }
}
