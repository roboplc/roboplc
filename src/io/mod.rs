pub use binrw;
use binrw::{BinRead, BinWrite};

use crate::Result;

#[cfg(feature = "eapi")]
pub mod eapi;
#[cfg(feature = "modbus")]
pub mod modbus;
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
