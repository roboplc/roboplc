use std::{fs::File, io::Write, path::Path};

use serde::{de::DeserializeOwned, Serialize};

use crate::{Error, Result};

enum Format {
    Json,
    Msgpack,
}

impl Format {
    fn from_path<P: AsRef<Path>>(path: P) -> Self {
        match path
            .as_ref()
            .extension()
            .map_or("", |ext| ext.to_str().unwrap())
        {
            "json" => Self::Json,
            _ => Self::Msgpack,
        }
    }
}

/// Load the state from a file. If "json" extension is specified, the state is loaded in JSON
/// format. All errors, including missing state file, must be handled by the caller.
pub fn load<S: DeserializeOwned, P: AsRef<Path>>(path: P) -> Result<S> {
    let format = Format::from_path(&path);
    let file = File::open(&path)?;
    let data = match format {
        Format::Json => serde_json::from_reader(file).map_err(Error::failed)?,
        Format::Msgpack => rmp_serde::from_read(file).map_err(Error::failed)?,
    };
    Ok(data)
}

/// Save the state to a file. If "json" extension is specified, the state is saved in JSON
/// format. Otherwise it is saved in MessagePack format.
pub fn save<S: Serialize, P: AsRef<Path>>(state: &S, path: P) -> Result<()> {
    let format = Format::from_path(&path);
    let mut file = File::create(&path)?;
    let data = match format {
        Format::Json => serde_json::to_vec(state).map_err(Error::failed)?,
        Format::Msgpack => rmp_serde::to_vec_named(state).map_err(Error::failed)?,
    };
    file.write_all(&data)?;
    Ok(())
}
