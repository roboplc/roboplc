use std::str::FromStr;

use crate::{Error, Result};

#[derive(Eq, PartialEq, Copy, Clone, Debug)]
pub enum Kind {
    Coil,
    Discrete,
    Input,
    Holding,
}

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
    if let Some(v) = r.strip_prefix('c') {
        Ok((Kind::Coil, v.parse()?))
    } else if let Some(v) = r.strip_prefix('d') {
        Ok((Kind::Discrete, v.parse()?))
    } else if let Some(v) = r.strip_prefix('i') {
        Ok((Kind::Input, v.parse()?))
    } else if let Some(v) = r.strip_prefix('h') {
        Ok((Kind::Holding, v.parse()?))
    } else {
        Err(Error::invalid_data(format!("invalid register kind: {}", r)))
    }
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
