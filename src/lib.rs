#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]
#![deny(missing_docs)]
use core::{fmt, num};
use std::io::Write;
use std::panic::PanicInfo;
use std::{env, sync::Arc, time::Duration};

use colored::Colorize as _;
use thread_rt::{RTParams, Scheduling};

pub use log::LevelFilter;
pub use rtsc::{DataChannel, DataPolicy};

#[cfg(feature = "locking-default")]
pub use parking_lot as locking;

#[cfg(feature = "locking-rt")]
pub use parking_lot_rt as locking;

#[cfg(all(feature = "locking-rt-safe", not(target_os = "linux")))]
pub use parking_lot_rt as locking;
#[cfg(all(feature = "locking-rt-safe", target_os = "linux"))]
pub use rtsc::pi as locking;

#[cfg(feature = "metrics")]
pub use metrics;

pub use rtsc::policy_channel_async;
pub use rtsc::time;

/// Wrapper around [`rtsc::buf`] with the chosen locking policy
pub mod buf {
    /// Type alias for [`rtsc::buf::DataBuffer`] with the chosen locking policy
    pub type DataBuffer = rtsc::buf::DataBuffer<crate::locking::RawMutex>;
}

/// Wrapper around [`rtsc::channel`] with the chosen locking policy
pub mod channel {

    /// Type alias for [`rtsc::channel::Sender`] with the chosen locking policy
    pub type Sender<T> =
        rtsc::channel::Sender<T, crate::locking::RawMutex, crate::locking::Condvar>;
    /// Type alias for [`rtsc::channel::Receiver`] with the chosen locking policy
    pub type Receiver<T> =
        rtsc::channel::Receiver<T, crate::locking::RawMutex, crate::locking::Condvar>;

    /// Function alias for [`rtsc::channel::bounded`] with the chosen locking policy
    #[inline]
    pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        rtsc::channel::bounded(capacity)
    }
}

/// Wrapper around [`rtsc::policy_channel`] with the chosen locking policy
pub mod policy_channel {
    use crate::DataDeliveryPolicy;

    /// Type alias for [`rtsc::policy_channel::Sender`] with the chosen locking policy
    pub type Sender<T> =
        rtsc::policy_channel::Sender<T, crate::locking::RawMutex, crate::locking::Condvar>;
    /// Type alias for [`rtsc::policy_channel::Receiver`] with the chosen locking policy
    pub type Receiver<T> =
        rtsc::policy_channel::Receiver<T, crate::locking::RawMutex, crate::locking::Condvar>;

    /// Function alias for [`rtsc::policy_channel::bounded`] with the chosen locking policy
    #[inline]
    pub fn bounded<T: DataDeliveryPolicy>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        rtsc::policy_channel::bounded(capacity)
    }

    /// Function alias for [`rtsc::policy_channel::ordered`] with the chosen locking policy
    #[inline]
    pub fn ordered<T: DataDeliveryPolicy>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        rtsc::policy_channel::ordered(capacity)
    }
}

/// Wrapper around [`rtsc::semaphore`] with the chosen locking policy
pub mod semaphore {
    /// Type alias for [`rtsc::semaphore::Semaphore`] with the chosen locking policy
    pub type Semaphore =
        rtsc::semaphore::Semaphore<crate::locking::RawMutex, crate::locking::Condvar>;
    /// Type alias for [`rtsc::semaphore::SemaphoreGuard`] with the chosen locking policy
    #[allow(clippy::module_name_repetitions)]
    pub type SemaphoreGuard =
        rtsc::semaphore::SemaphoreGuard<crate::locking::RawMutex, crate::locking::Condvar>;
}

pub use rtsc::data_policy::{DataDeliveryPolicy, DeliveryPolicy};

/// Reliable TCP/Serial communications
pub mod comm;
/// Controller and workers
pub mod controller;
/// In-process data communication pub/sub hub, synchronous edition
pub mod hub;
/// In-process data communication pub/sub hub, asynchronous edition
#[cfg(feature = "async")]
pub mod hub_async;
/// I/O
pub mod io;
/// Task supervisor to manage real-time threads
pub mod supervisor;
/// Real-time thread functions to work with [`supervisor::Supervisor`] and standalone, Linux only
pub mod thread_rt;

/// The crate result type
pub type Result<T> = std::result::Result<T, Error>;

/// The crate error type
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// the channel is full and the value can not be sent
    #[error("channel full")]
    ChannelFull,
    /// the channel is full, an optional value is skipped. the error can be ignored but should be
    /// logged
    #[error("channel message skipped")]
    ChannelSkipped,
    /// The channel is closed (all transmitters/receivers gone)
    #[error("channel closed")]
    ChannelClosed,
    /// Receive attempt failed because the channel is empty
    #[error("channel empty")]
    ChannelEmpty,
    /// Hub send errors
    #[error("hub send error {0}")]
    HubSend(Box<Error>),
    /// Hub client with the given name is already registered
    #[error("hub client already registered: {0}")]
    HubAlreadyRegistered(Arc<str>),
    /// Timeouts
    #[error("timed out")]
    Timeout,
    /// Standard I/O errors
    #[error("I/O error: {0}")]
    IO(#[from] std::io::Error),
    /// Non-standard I/O errors
    #[error("Communication error: {0}")]
    Comm(String),
    /// 3rd party API errors
    #[error("API error {0}: {1}")]
    API(String, i64),
    /// Real-time engine error: unable to get the system thread id
    #[error("RT SYS_gettid {0}")]
    RTGetTId(libc::c_int),
    /// Real-time engine error: unable to set the thread scheduler affinity
    #[error("RT sched_setaffinity {0}")]
    RTSchedSetAffinity(libc::c_int),
    /// Real-time engine error: unable to set the thread scheduler policy
    #[error("RT sched_setscheduler {0}")]
    RTSchedSetSchduler(libc::c_int),
    /// Supervisor error: task name is not specified in the thread builder
    #[error("Task name must be specified when spawning by a supervisor")]
    SupervisorNameNotSpecified,
    /// Supervisor error: task with the given name is already registered
    #[error("Task already registered: `{0}`")]
    SupervisorDuplicateTask(String),
    /// Supervisor error: task with the given name is not found
    #[error("Task not found")]
    SupervisorTaskNotFound,
    /// Invalid data receied / parameters provided
    #[error("Invalid data")]
    InvalidData(String),
    /// [binrw](https://crates.io/crates/binrw) crate errors
    #[error("binrw {0}")]
    BinRw(String),
    /// The requested operation is not implemented
    #[error("not implemented")]
    Unimplemented,
    /// This error never happens and is used as a compiler hint only
    #[error("never happens")]
    Infallible(#[from] std::convert::Infallible),
    /// All other errors
    #[error("operation failed: {0}")]
    Failed(String),
}

impl From<rtsc::Error> for Error {
    fn from(err: rtsc::Error) -> Self {
        match err {
            rtsc::Error::ChannelFull => Error::ChannelFull,
            rtsc::Error::ChannelSkipped => Error::ChannelSkipped,
            rtsc::Error::ChannelClosed => Error::ChannelClosed,
            rtsc::Error::ChannelEmpty => Error::ChannelEmpty,
            rtsc::Error::Unimplemented => Error::Unimplemented,
            rtsc::Error::Timeout => Error::Timeout,
            rtsc::Error::InvalidData(msg) => Error::InvalidData(msg),
            rtsc::Error::Failed(msg) => Error::Failed(msg),
        }
    }
}

impl From<Error> for rtsc::Error {
    fn from(err: Error) -> Self {
        match err {
            Error::ChannelFull => rtsc::Error::ChannelFull,
            Error::ChannelSkipped => rtsc::Error::ChannelSkipped,
            Error::ChannelClosed => rtsc::Error::ChannelClosed,
            Error::ChannelEmpty => rtsc::Error::ChannelEmpty,
            Error::Unimplemented => rtsc::Error::Unimplemented,
            Error::Timeout => rtsc::Error::Timeout,
            Error::InvalidData(msg) => rtsc::Error::InvalidData(msg),
            _ => rtsc::Error::Failed(err.to_string()),
        }
    }
}

macro_rules! impl_error {
    ($t: ty, $key: ident) => {
        impl From<$t> for Error {
            fn from(err: $t) -> Self {
                Error::$key(err.to_string())
            }
        }
    };
}

#[cfg(feature = "modbus")]
impl_error!(rmodbus::ErrorKind, Comm);
impl_error!(oneshot::RecvError, Comm);
impl_error!(num::ParseIntError, InvalidData);
impl_error!(num::ParseFloatError, InvalidData);
impl_error!(binrw::Error, BinRw);

impl Error {
    /// Returns true if the data is skipped
    pub fn is_data_skipped(&self) -> bool {
        matches!(self, Error::ChannelSkipped)
    }
    /// Creates new invalid data error
    pub fn invalid_data<S: fmt::Display>(msg: S) -> Self {
        Error::InvalidData(msg.to_string())
    }
    /// Creates new I/O error (for non-standard I/O)
    pub fn io<S: fmt::Display>(msg: S) -> Self {
        Error::Comm(msg.to_string())
    }
    /// Creates new function failed error
    pub fn failed<S: fmt::Display>(msg: S) -> Self {
        Error::Failed(msg.to_string())
    }
}

/// Immediately kills the current process and all its subprocesses with a message to stderr
pub fn critical(msg: &str) -> ! {
    eprintln!("{}", msg.red().bold());
    thread_rt::suicide_myself(Duration::from_secs(0), false);
    std::process::exit(1);
}

/// Terminates the current process and all its subprocesses in the specified period of time with
/// SIGKILL command. Useful if a process is unable to shut it down gracefully within a specified
/// period of time.
///
/// Prints warnings to STDOUT if warn is true
pub fn suicide(delay: Duration, warn: bool) {
    let mut builder = thread_rt::Builder::new().name("suicide").rt_params(
        RTParams::new()
            .set_priority(99)
            .set_scheduling(Scheduling::FIFO)
            .set_cpu_ids(&[0]),
    );
    builder.park_on_errors = true;
    let res = builder.spawn(move || {
        thread_rt::suicide_myself(delay, warn);
    });
    if res.is_err() {
        std::thread::spawn(move || {
            thread_rt::suicide_myself(delay, warn);
        });
    };
}

#[cfg(feature = "rvideo")]
pub use rvideo;

#[cfg(feature = "rflow")]
pub use rflow;

#[cfg(feature = "rvideo")]
/// Serves the default [`rvideo`] server at TCP port `0.0.0.0:3001`
pub fn serve_rvideo() -> std::result::Result<(), rvideo::Error> {
    rvideo::serve("0.0.0.0:3001").map_err(Into::into)
}

#[cfg(feature = "rflow")]
/// Serves the default [`rflow`] server at TCP port `0.0.0.0:4001`
pub fn serve_rflow() -> std::result::Result<(), rflow::Error> {
    rflow::serve("0.0.0.0:4001").map_err(Into::into)
}

/// Returns [Prometheus metrics exporter
/// builder](https://docs.rs/metrics-exporter-prometheus/)
///
/// # Example
///
/// ```rust,no_run
/// roboplc::metrics_exporter()
///   .set_bucket_duration(std::time::Duration::from_secs(300)).unwrap()
///   .install().unwrap();
/// ```
#[cfg(feature = "metrics")]
pub fn metrics_exporter() -> metrics_exporter_prometheus::PrometheusBuilder {
    metrics_exporter_prometheus::PrometheusBuilder::new()
}

/// Installs Prometheus metrics exporter together with [Scope
/// exporter](https://docs.rs/metrics-exporter-scope)
#[cfg(feature = "metrics")]
pub fn metrics_exporter_install(
    builder: metrics_exporter_prometheus::PrometheusBuilder,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (prometheus_exporter, prometheus_exporter_fut) = {
        let _g = runtime.enter();
        builder.build().map_err(Error::failed)?
    };
    metrics_exporter_scope::ScopeBuilder::new()
        .with_fallback(Box::new(prometheus_exporter))
        .install()
        .map_err(Error::failed)?;
    std::thread::Builder::new()
        .name("metrics_exporter".to_owned())
        .spawn(move || {
            runtime.block_on(prometheus_exporter_fut).unwrap();
        })?;
    Ok(())
}

/// Sets panic handler to immediately kill the process and its childs with SIGKILL. The process is
/// killed when panic happens in ANY thread
pub fn setup_panic() {
    std::panic::set_hook(Box::new(move |info: &PanicInfo| {
        panic(info);
    }));
}

fn panic(info: &PanicInfo) -> ! {
    eprintln!("{}", info.to_string().red().bold());
    thread_rt::suicide_myself(Duration::from_secs(0), false);
    // never happens
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Returns true if started in production mode (as a systemd unit)
pub fn is_production() -> bool {
    env::var("INVOCATION_ID").map_or(false, |v| !v.is_empty())
}

/// Configures stdout logger with the given filter. If started in production mode, does not logs
/// timestamps
pub fn configure_logger(filter: LevelFilter) {
    let mut builder = env_logger::Builder::new();
    builder.target(env_logger::Target::Stdout);
    builder.filter_level(filter);
    if is_production() && !env::var("ROBOPLC_MODE").map_or(false, |m| m == "exec") {
        builder.format(|buf, record| writeln!(buf, "{} {}", record.level(), record.args()));
    }
    builder.init();
}

/// Reload the current executable (performs execvp syscall, Linux only)
#[cfg(target_os = "linux")]
pub fn reload_executable() -> Result<()> {
    let mut current_exe = std::env::current_exe()?;
    // handle a case if the executable is deleted
    let fname = current_exe
        .file_name()
        .ok_or_else(|| Error::Failed("No file name".to_owned()))?
        .to_string_lossy()
        .trim_end_matches(" (deleted)")
        .to_owned();
    current_exe = current_exe.with_file_name(fname);
    std::os::unix::process::CommandExt::exec(&mut std::process::Command::new(current_exe));
    Ok(())
}

/// Reload the current executable (performs execvp syscall, Linux only)
#[cfg(not(target_os = "linux"))]
pub fn reload_executable() -> Result<()> {
    Err(Error::Unimplemented)
}

/// Prelude module
pub mod prelude {
    pub use super::suicide;
    pub use crate::controller::*;
    pub use crate::hub::prelude::*;
    pub use crate::io::prelude::*;
    pub use crate::supervisor::prelude::*;
    pub use crate::time::DurationRT;
    pub use bma_ts::{Monotonic, Timestamp};
    pub use rtsc::DataPolicy;
    pub use std::time::Duration;
}
