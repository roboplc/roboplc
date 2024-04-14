//!
//! Contains Modbus client/server implementations.
//!
//! Examples: [modbus
//! master(client)](https://github.com/roboplc/roboplc/blob/main/examples/modbus-master.rs),
//! [modbus slave(server)](https://github.com/roboplc/roboplc/blob/main/examples/modbus-slave.rs)
use std::io::Cursor;

use crate::comm::{Client, Protocol};
use crate::{Error, Result};
use binrw::{BinRead, BinWrite};
#[allow(clippy::module_name_repetitions)]
pub use regs::{Kind as ModbusRegisterKind, Register as ModbusRegister};
use rmodbus::guess_response_frame_len;
use rmodbus::{client::ModbusRequest as RModbusRequest, ModbusProto};
#[allow(clippy::module_name_repetitions)]
pub use server::{ModbusServer, ModbusServerMapping};

use super::IoMapping;

mod regs;
mod server;

pub mod prelude {
    pub use super::{
        ModbusMapping, ModbusMappingOptions, ModbusRegister, ModbusRegisterKind, ModbusServer,
        ModbusServerMapping,
    };
}

/// Swaps endianess of floating point numbers in case of non-standard IEEE 754 layout.
pub trait SwapModbusEndianess {
    fn to_swapped_modbus_endianness(&self) -> Self;
}

impl SwapModbusEndianess for f32 {
    fn to_swapped_modbus_endianness(&self) -> Self {
        let b = self.to_be_bytes();
        Self::from_be_bytes([b[2], b[3], b[0], b[1]])
    }
}

impl SwapModbusEndianess for f64 {
    fn to_swapped_modbus_endianness(&self) -> Self {
        let b = self.to_be_bytes();
        Self::from_be_bytes([b[6], b[7], b[4], b[5], b[2], b[3], b[0], b[1]])
    }
}

impl From<Protocol> for ModbusProto {
    fn from(value: Protocol) -> Self {
        match value {
            Protocol::Tcp => ModbusProto::TcpUdp,
            Protocol::Serial => ModbusProto::Rtu,
        }
    }
}

/// Mapping options for Modbus client
#[allow(clippy::module_name_repetitions)]
#[derive(Clone)]
pub struct ModbusMappingOptions {
    bulk_write: bool,
}

impl ModbusMappingOptions {
    pub fn new() -> Self {
        Self { bulk_write: true }
    }
    pub fn bulk_write(mut self, value: bool) -> Self {
        self.bulk_write = value;
        self
    }
}

impl Default for ModbusMappingOptions {
    fn default() -> Self {
        Self { bulk_write: true }
    }
}

/// Mapping for Modbus client
#[allow(clippy::module_name_repetitions)]
pub struct ModbusMapping {
    client: Client,
    unit_id: u8,
    register: ModbusRegister,
    count: u16,
    request_id: u16,
    buf: Vec<u8>,
    rest_buf: Vec<u8>,
    data_buf: Vec<u8>,
    options: ModbusMappingOptions,
}

impl ModbusMapping {
    pub fn create<R>(client: &Client, unit_id: u8, register: R, count: u16) -> Result<Self>
    where
        R: TryInto<ModbusRegister>,
        Error: From<<R as TryInto<ModbusRegister>>::Error>,
    {
        Ok(Self {
            client: client.clone(),
            unit_id,
            register: register.try_into()?,
            count,
            request_id: 1,
            // pre-allocate buffers
            buf: Vec::with_capacity(256),
            rest_buf: Vec::with_capacity(256),
            data_buf: vec![],
            options: <_>::default(),
        })
    }
    pub fn with_options(mut self, options: ModbusMappingOptions) -> Self {
        self.options = options;
        self
    }
}

macro_rules! prepare_transaction {
    ($self: expr) => {{
        let mut mreq = RModbusRequest::new($self.unit_id, $self.client.protocol().into());
        mreq.tr_id = $self.request_id;
        $self.request_id += 1;
        $self.buf.truncate(0);
        mreq
    }};
}

macro_rules! communicate {
    ($self: expr) => {
        $self.client.write(&$self.buf)?;
        let mut buf = [0u8; 6];
        $self.client.read_exact(&mut buf)?;
        $self.buf.truncate(0);
        $self.buf.extend(buf);
        let len = guess_response_frame_len(&buf, $self.client.protocol().into())?;
        if len > 6 {
            $self.rest_buf.resize(usize::from(len - 6), 0);
            $self.client.read_exact(&mut $self.rest_buf)?;
            $self.buf.extend(&$self.rest_buf);
        }
    };
}

impl IoMapping for ModbusMapping {
    type Options = ModbusMappingOptions;
    fn read<T>(&mut self) -> Result<T>
    where
        T: for<'a> BinRead<Args<'a> = ()>,
    {
        let _lock = self.client.lock();
        let mut mreq = prepare_transaction!(self);
        match self.register.kind {
            ModbusRegisterKind::Coil => {
                mreq.generate_get_coils(self.register.offset, self.count, &mut self.buf)?;
            }
            ModbusRegisterKind::Discrete => {
                mreq.generate_get_discretes(self.register.offset, self.count, &mut self.buf)?;
            }
            ModbusRegisterKind::Input => {
                mreq.generate_get_inputs(self.register.offset, self.count, &mut self.buf)?;
            }
            ModbusRegisterKind::Holding => {
                mreq.generate_get_holdings(self.register.offset, self.count, &mut self.buf)?;
            }
        };
        communicate!(self);
        match self.register.kind {
            ModbusRegisterKind::Coil | ModbusRegisterKind::Discrete => {
                self.data_buf.truncate(0);
                mreq.parse_bool_u8(&self.buf, &mut self.data_buf)?;
                let mut reader = Cursor::new(&self.data_buf);
                T::read_be(&mut reader).map_err(Into::into)
            }
            ModbusRegisterKind::Input | ModbusRegisterKind::Holding => {
                let data = mreq.parse_slice(&self.buf)?;
                if data.is_empty() {
                    return Err(Error::invalid_data("invalid modbus response"));
                }
                let mut reader = Cursor::new(data);
                T::read_be(&mut reader).map_err(Into::into)
            }
        }
    }

    fn write<T>(&mut self, value: T) -> Result<()>
    where
        T: for<'a> BinWrite<Args<'a> = ()>,
    {
        let _lock = self.client.lock();
        let mut data_buf = Cursor::new(&mut self.data_buf);
        value.write_be(&mut data_buf)?;
        if self.options.bulk_write {
            let mut mreq = prepare_transaction!(self);
            match self.register.kind {
                ModbusRegisterKind::Coil => {
                    mreq.generate_set_coils_bulk(
                        self.register.offset,
                        &self.data_buf,
                        &mut self.buf,
                    )?;
                }
                ModbusRegisterKind::Holding => {
                    mreq.generate_set_holdings_bulk_from_slice(
                        self.register.offset,
                        &self.data_buf,
                        &mut self.buf,
                    )?;
                }
                ModbusRegisterKind::Discrete | ModbusRegisterKind::Input => {
                    return Err(Error::IO(
                        "unsupported modbus register kind for writing".to_owned(),
                    ));
                }
            }
            communicate!(self);
            mreq.parse_ok(&self.buf)?;
        } else {
            let mut i = 0;
            for offset in self.register.offset..self.register.offset + self.count {
                let mut mreq = prepare_transaction!(self);
                match self.register.kind {
                    ModbusRegisterKind::Coil => {
                        mreq.generate_set_coil(
                            offset,
                            self.data_buf.get(i).copied().unwrap_or_default(),
                            &mut self.buf,
                        )?;
                        i += 1;
                    }
                    ModbusRegisterKind::Holding => {
                        let high = self.data_buf.get(i).copied().unwrap_or_default();
                        let low = self.data_buf.get(i + 1).copied().unwrap_or_default();
                        let value: u16 = u16::from(high) << 8 | u16::from(low);
                        mreq.generate_set_holding(offset, value, &mut self.buf)?;
                        i += 2;
                    }
                    ModbusRegisterKind::Discrete | ModbusRegisterKind::Input => {
                        return Err(Error::IO(
                            "unsupported modbus register kind for writing".to_owned(),
                        ));
                    }
                }
                communicate!(self);
                mreq.parse_ok(&self.buf)?;
            }
        }
        Ok(())
    }
}
