use crate::io::{modbus::ModbusRegister, IoMapping};
use crate::locking::{Mutex, MutexGuard};
use crate::semaphore::Semaphore;
use crate::{
    comm::{self, Protocol},
    Error, Result,
};
use binrw::{BinRead, BinWrite};
use rmodbus::{
    server::{context::ModbusContext, storage::ModbusStorage, ModbusFrame},
    ModbusFrameBuf, ModbusProto,
};
use serial::SystemPort;
use std::time::Duration;
use std::{
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
};
use tracing::error;

use super::ModbusRegisterKind;

enum Server {
    Tcp(TcpListener),
    Serial(SystemPort),
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn handle_client<
    T: Read + Write,
    const C: usize,
    const D: usize,
    const I: usize,
    const H: usize,
>(
    mut client: T,
    unit: u8,
    storage: Arc<Mutex<ModbusStorage<C, D, I, H>>>,
    modbus_proto: ModbusProto,
    allow_write: &AllowFn,
) -> Result<()> {
    let mut buf: ModbusFrameBuf = [0; 256];
    let mut response = Vec::with_capacity(256);
    loop {
        if client.read(&mut buf).unwrap_or(0) == 0 {
            break;
        }
        response.truncate(0);
        let mut frame = ModbusFrame::new(unit, &buf, modbus_proto, &mut response);
        frame.parse().map_err(Error::io)?;
        if frame.processing_required {
            if frame.readonly {
                frame.process_read(&*storage.lock()).map_err(Error::io)?;
            } else {
                let (process, _guard) = if let Some(changes) = frame.changes() {
                    let (kind, range) = match changes {
                        rmodbus::server::Changes::Coils { reg, count } => {
                            (ModbusRegisterKind::Coil, reg..reg + count)
                        }
                        rmodbus::server::Changes::Holdings { reg, count } => {
                            (ModbusRegisterKind::Holding, reg..reg + count)
                        }
                    };
                    match allow_write(kind, range) {
                        WritePermission::Allow => (true, None),
                        WritePermission::AllowLock(guard) => (true, Some(guard)),
                        WritePermission::Deny => (false, None),
                    }
                } else {
                    (true, None)
                };
                if process {
                    frame
                        .process_write(&mut *storage.lock())
                        .map_err(Error::io)?;
                } else {
                    frame.set_modbus_error_if_unset(&rmodbus::ErrorKind::NegativeAcknowledge)?;
                }
            }
        }
        if frame.response_required {
            frame.finalize_response().map_err(Error::io)?;
            client.write_all(&response).map_err(Error::io)?;
        }
    }
    Ok(())
}

/// Function to block certain context storage
pub type AllowFn = fn(ModbusRegisterKind, std::ops::Range<u16>) -> WritePermission;

/// Context storage write permission
pub enum WritePermission {
    /// Write is allowed.
    Allow,
    /// Write is allowed with a lock, the lock is released after the write operation.
    AllowLock(MutexGuard<'static, ()>),
    /// Write is forbidden.
    Deny,
}

impl From<bool> for WritePermission {
    fn from(value: bool) -> Self {
        if value {
            Self::Allow
        } else {
            Self::Deny
        }
    }
}

impl From<MutexGuard<'static, ()>> for WritePermission {
    fn from(guard: MutexGuard<'static, ()>) -> Self {
        Self::AllowLock(guard)
    }
}

/// Modbus server. Requires to be run in a separate thread manually.
#[allow(clippy::module_name_repetitions)]
pub struct ModbusServer<const C: usize, const D: usize, const I: usize, const H: usize> {
    storage: Arc<Mutex<ModbusStorage<C, D, I, H>>>,
    unit: u8,
    server: Server,
    timeout: Duration,
    semaphore: Semaphore,
    allow_external_write_fn: Arc<AllowFn>,
}
impl<const C: usize, const D: usize, const I: usize, const H: usize> ModbusServer<C, D, I, H> {
    /// Creates new Modbus server
    pub fn bind(
        protocol: Protocol,
        unit: u8,
        path: &str,
        timeout: Duration,
        max_workers: usize,
    ) -> Result<Self> {
        let server = match protocol {
            Protocol::Tcp => Server::Tcp(TcpListener::bind(path)?),
            Protocol::Serial => Server::Serial(comm::serial::open(&path.parse()?, timeout)?),
        };
        Ok(Self {
            storage: <_>::default(),
            unit,
            server,
            timeout,
            semaphore: Semaphore::new(max_workers),
            allow_external_write_fn: Arc::new(|_, _| WritePermission::Allow),
        })
    }
    /// Set a function which checks if an external client write operation is allowed.
    /// The function allows to block a client until a certain storage context range is processed by
    /// an internal task.
    pub fn set_allow_external_write_fn(&mut self, f: AllowFn) {
        self.allow_external_write_fn = f.into();
    }
    /// Creates a new mapping for the server storage context.
    pub fn mapping(&self, register: ModbusRegister, count: u16) -> ModbusServerMapping<C, D, I, H> {
        let buf_capacity = match register.kind {
            ModbusRegisterKind::Coil | ModbusRegisterKind::Discrete => usize::from(count),
            ModbusRegisterKind::Input | ModbusRegisterKind::Holding => usize::from(count) * 2,
        };
        ModbusServerMapping {
            storage: self.storage.clone(),
            register,
            count,
            data_buf: Vec::with_capacity(buf_capacity),
        }
    }
    /// Returns a reference to the internal storage.
    pub fn storage(&self) -> Arc<Mutex<ModbusStorage<C, D, I, H>>> {
        self.storage.clone()
    }
    /// Runs the server. This function blocks the current thread.
    pub fn serve(&mut self) -> Result<()> {
        let timeout = self.timeout;
        let unit = self.unit;
        match self.server {
            Server::Tcp(ref server) => loop {
                let permission = self.semaphore.acquire();
                let (stream, addr) = server.accept()?;
                if let Err(e) = prepare_tcp_stream(&stream, timeout) {
                    error!(%addr, %e, "error preparing tcp stream");
                    continue;
                }
                let storage = self.storage.clone();
                let allow_write = self.allow_external_write_fn.clone();
                thread::spawn(move || {
                    let _permission = permission;
                    if let Err(error) =
                        handle_client(stream, unit, storage, ModbusProto::TcpUdp, &allow_write)
                    {
                        error!(%addr, %error, "error handling Modbus client");
                    }
                });
            },
            Server::Serial(ref mut serial) => loop {
                if let Err(e) = handle_client(
                    &mut *serial,
                    unit,
                    self.storage.clone(),
                    ModbusProto::Rtu,
                    &self.allow_external_write_fn,
                ) {
                    error!(%e, "error handling Modbus client");
                }
            },
        }
    }
}

fn prepare_tcp_stream(stream: &TcpStream, timeout: Duration) -> Result<()> {
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_nodelay(true)?;
    Ok(())
}

/// Server storage context mapping.
pub struct ModbusServerMapping<const C: usize, const D: usize, const I: usize, const H: usize> {
    storage: Arc<Mutex<ModbusStorage<C, D, I, H>>>,
    register: ModbusRegister,
    count: u16,
    data_buf: Vec<u8>,
}

impl<const C: usize, const D: usize, const I: usize, const H: usize> IoMapping
    for ModbusServerMapping<C, D, I, H>
{
    type Options = ();

    fn read<T>(&mut self) -> Result<T>
    where
        T: for<'a> BinRead<Args<'a> = ()>,
    {
        self.data_buf.truncate(0);
        match self.register.kind {
            ModbusRegisterKind::Coil => self
                .storage
                .lock()
                .get_coils_as_u8_bytes(self.register.offset, self.count, &mut self.data_buf)
                .map_err(Error::io)?,
            ModbusRegisterKind::Discrete => self
                .storage
                .lock()
                .get_discretes_as_u8_bytes(self.register.offset, self.count, &mut self.data_buf)
                .map_err(Error::io)?,
            ModbusRegisterKind::Input => self
                .storage
                .lock()
                .get_inputs_as_u8(self.register.offset, self.count, &mut self.data_buf)
                .map_err(Error::io)?,
            ModbusRegisterKind::Holding => self
                .storage
                .lock()
                .get_holdings_as_u8(self.register.offset, self.count, &mut self.data_buf)
                .map_err(Error::io)?,
        };
        let mut reader = Cursor::new(&self.data_buf);
        T::read_be(&mut reader).map_err(Into::into)
    }

    fn write<T>(&mut self, value: T) -> Result<()>
    where
        T: for<'a> BinWrite<Args<'a> = ()>,
    {
        let mut data_buf = Cursor::new(&mut self.data_buf);
        value.write_be(&mut data_buf)?;
        macro_rules! check_data_len_bool {
            () => {
                if self.data_buf.len() > self.count.into() {
                    return Err(Error::io("invalid data length"));
                }
            };
        }
        macro_rules! check_data_len_u16 {
            () => {
                if self.data_buf.len() > usize::from(self.count) * 2 {
                    return Err(Error::io("invalid data length"));
                }
            };
        }
        match self.register.kind {
            ModbusRegisterKind::Coil => {
                check_data_len_bool!();
                self.storage
                    .lock()
                    .set_coils_from_u8_bytes(self.register.offset, &self.data_buf)
                    .map_err(Error::io)?;
            }
            ModbusRegisterKind::Discrete => {
                check_data_len_bool!();
                self.storage
                    .lock()
                    .set_discretes_from_u8_bytes(self.register.offset, &self.data_buf)
                    .map_err(Error::io)?;
            }
            ModbusRegisterKind::Input => {
                check_data_len_u16!();
                self.storage
                    .lock()
                    .set_inputs_from_u8(self.register.offset, &self.data_buf)
                    .map_err(Error::io)?;
            }
            ModbusRegisterKind::Holding => {
                check_data_len_u16!();
                self.storage
                    .lock()
                    .set_holdings_from_u8(self.register.offset, &self.data_buf)
                    .map_err(Error::io)?;
            }
        };
        Ok(())
    }
}
