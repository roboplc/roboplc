//!
//! The module provides mapping for various protocols. Structural mapping is built on top of
//! the [binrw](https://crates.io/crates/binrw) crate.
pub use binrw;
use binrw::{BinRead, BinWrite};

use crate::Result;

#[cfg(feature = "eapi")]
/// EVA ICS local bus API
pub mod eapi;
#[cfg(feature = "modbus")]
/// Modbus communication
pub mod modbus;
/// Raw UDP communication
pub mod raw_udp;

#[allow(clippy::module_name_repetitions)]
pub trait IoMapping {
    type Options;
    fn read<T>(&mut self) -> Result<T>
    where
        T: for<'a> BinRead<Args<'a> = ()>;
    fn write<T>(&mut self, value: T) -> Result<()>
    where
        T: for<'a> BinWrite<Args<'a> = ()>;
}

pub mod prelude {
    pub use super::IoMapping as _;
    pub use binrw::prelude::*;
}
