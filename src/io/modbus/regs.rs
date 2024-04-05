use std::str::FromStr;

use crate::{Error, Result};

/// A Modbus register kind.
#[derive(Eq, PartialEq, Copy, Clone, Debug)]
pub enum Kind {
    Coil,
    Discrete,
    Input,
    Holding,
}

/// A Modbus register type, contains the kind and the offset.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Register {
    pub kind: Kind,
    pub offset: u16,
}

impl Register {
    pub fn new(kind: Kind, offset: u16) -> Self {
        Self { kind, offset }
    }
}

fn parse_kind_offset(r: &str) -> Result<(Kind, u16)> {
    macro_rules! err {
        () => {
            Error::invalid_data(format!("invalid register: {}", r))
        };
    }
    let mut chars = r.chars();
    let kind = match chars.next().ok_or_else(|| err!())? {
        'c' => Kind::Coil,
        'd' => Kind::Discrete,
        'i' => Kind::Input,
        'h' => Kind::Holding,
        _ => return Err(Error::invalid_data(format!("invalid register kind: {}", r))),
    };
    let o = if chars.next().ok_or_else(|| err!())? == '@' {
        2
    } else {
        1
    };
    let offset = r[o..].parse().map_err(|_| err!())?;
    Ok((kind, offset))
}

impl FromStr for Register {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        let (kind, offset) = parse_kind_offset(s)?;
        Ok(Register { kind, offset })
    }
}

impl TryFrom<&str> for Register {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self> {
        s.parse()
    }
}
