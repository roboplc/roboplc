use std::{fs::File, io::Write, path::Path};

use serde::{de::DeserializeOwned, Serialize};

use crate::{Error, Result};

enum Format {
    #[cfg(feature = "json")]
    Json,
    #[cfg(feature = "msgpack")]
    Msgpack,
}

impl Format {
    #[allow(clippy::unnecessary_wraps)]
    fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        match path
            .as_ref()
            .extension()
            .map_or("", |ext| ext.to_str().unwrap())
        {
            #[cfg(feature = "json")]
            "json" => Ok(Self::Json),
            #[cfg(not(feature = "json"))]
            "json" => Err(Error::Unimplemented),
            #[cfg(feature = "msgpack")]
            _ => Ok(Self::Msgpack),
            #[cfg(not(feature = "msgpack"))]
            _ => Err(Error::Unimplemented),
        }
    }
}

/// Load the state from a file. If "json" extension is specified, the state is loaded from JSON
/// format (requires crate 'json' feature), otherwise from MessagePack (requires crate 'msgpack'
/// feature). All errors, including missing state file, must be handled by the caller.
pub fn load<S: DeserializeOwned, P: AsRef<Path>>(path: P) -> Result<S> {
    let format = Format::from_path(&path)?;
    let file = File::open(&path)?;
    let data = match format {
        #[cfg(feature = "json")]
        Format::Json => serde_json::from_reader(file).map_err(Error::failed)?,
        #[cfg(feature = "msgpack")]
        Format::Msgpack => rmp_serde::from_read(file).map_err(Error::failed)?,
    };
    Ok(data)
}

/// Save the state to a file. If "json" extension is specified, the state is saved in JSON format
/// (requires crate 'json' feature), otherwise in MessagePack (requires crate 'msgpack' feature).
pub fn save<S: Serialize, P: AsRef<Path>>(path: P, state: &S) -> Result<()> {
    let format = Format::from_path(&path)?;
    let mut file = File::create(&path)?;
    let data = match format {
        #[cfg(feature = "json")]
        Format::Json => serde_json::to_vec(state).map_err(Error::failed)?,
        #[cfg(feature = "msgpack")]
        Format::Msgpack => rmp_serde::to_vec_named(state).map_err(Error::failed)?,
    };
    file.write_all(&data)?;
    Ok(())
}
