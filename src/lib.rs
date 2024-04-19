#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]
use core::{fmt, num};
use std::io::Write;
use std::panic::PanicInfo;
use std::{env, mem, str::FromStr, sync::Arc, time::Duration};

use colored::Colorize as _;
use thread_rt::{RTParams, Scheduling};

pub use log::LevelFilter;
pub use roboplc_derive::DataPolicy;

#[cfg(feature = "metrics")]
pub use metrics;

/// Event buffers
pub mod buf;
/// Reliable TCP/Serial communications
pub mod comm;
/// Controller and workers
pub mod controller;
/// In-process data communication pub/sub hub, synchronous edition
pub mod hub;
/// In-process data communication pub/sub hub, asynchronous edition
pub mod hub_async;
/// I/O
pub mod io;
/// Policy-based channels, synchronous edition
pub mod pchannel;
/// Policy-based channels, asynchronous edition
pub mod pchannel_async;
/// Policy-based data storages
pub mod pdeque;
/// A lighweight real-time safe semaphore
pub mod semaphore;
/// Task supervisor to manage real-time threads
pub mod supervisor;
/// Real-time thread functions to work with [`supervisor::Supervisor`] and standalone
pub mod thread_rt;
/// Various time tools for real-time applications
pub mod time;
/// A memory cell with an expiring value
pub mod ttlcell;

pub type Result<T> = std::result::Result<T, Error>;

/// The crate error type
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
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
    /// I/O and threading errors
    #[error("I/O error: {0}")]
    IO(String),
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

macro_rules! impl_error {
    ($t: ty, $key: ident) => {
        impl From<$t> for Error {
            fn from(err: $t) -> Self {
                Error::$key(err.to_string())
            }
        }
    };
}

impl_error!(std::io::Error, IO);
#[cfg(feature = "modbus")]
impl_error!(rmodbus::ErrorKind, IO);
impl_error!(oneshot::RecvError, IO);
impl_error!(num::ParseIntError, InvalidData);
impl_error!(num::ParseFloatError, InvalidData);
impl_error!(binrw::Error, BinRw);

impl Error {
    pub fn is_data_skipped(&self) -> bool {
        matches!(self, Error::ChannelSkipped)
    }
    pub fn invalid_data<S: fmt::Display>(msg: S) -> Self {
        Error::InvalidData(msg.to_string())
    }
    pub fn io<S: fmt::Display>(msg: S) -> Self {
        Error::IO(msg.to_string())
    }
    pub fn failed<S: fmt::Display>(msg: S) -> Self {
        Error::Failed(msg.to_string())
    }
}

/// Data delivery policies, used by [`hub::Hub`], [`pchannel::Receiver`] and [`pdeque::Deque`]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum DeliveryPolicy {
    #[default]
    /// always deliver, fail if no room (default)
    Always,
    /// always deliver, drop the previous if no room (act as a ring-buffer)
    Latest,
    /// skip delivery if no room
    Optional,
    /// always deliver the frame but always in a single copy (latest)
    Single,
    /// deliver a single latest copy, skip if no room
    SingleOptional,
}

impl FromStr for DeliveryPolicy {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "always" => Ok(DeliveryPolicy::Always),
            "optional" => Ok(DeliveryPolicy::Optional),
            "single" => Ok(DeliveryPolicy::Single),
            "single-optional" => Ok(DeliveryPolicy::SingleOptional),
            _ => Err(Error::invalid_data(s)),
        }
    }
}

impl fmt::Display for DeliveryPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                DeliveryPolicy::Always => "always",
                DeliveryPolicy::Latest => "latest",
                DeliveryPolicy::Optional => "optional",
                DeliveryPolicy::Single => "single",
                DeliveryPolicy::SingleOptional => "single-optional",
            }
        )
    }
}

/// Implements delivery policies for own data types
pub trait DataDeliveryPolicy
where
    Self: Sized,
{
    /// Delivery policy, the default is [`DeliveryPolicy::Always`]
    fn delivery_policy(&self) -> DeliveryPolicy {
        DeliveryPolicy::Always
    }
    /// Priority, for ordered, lower is better, the default is 100
    fn priority(&self) -> usize {
        100
    }
    /// Has equal kind with other
    ///
    /// (default: check enum discriminant)
    fn eq_kind(&self, other: &Self) -> bool {
        mem::discriminant(self) == mem::discriminant(other)
    }
    /// If a frame expires during storing/delivering, it is not delivered
    fn is_expired(&self) -> bool {
        false
    }
    #[doc(hidden)]
    fn is_delivery_policy_single(&self) -> bool {
        let dp = self.delivery_policy();
        dp == DeliveryPolicy::Single || dp == DeliveryPolicy::SingleOptional
    }
    #[doc(hidden)]
    fn is_delivery_policy_optional(&self) -> bool {
        let dp = self.delivery_policy();
        dp == DeliveryPolicy::Optional || dp == DeliveryPolicy::SingleOptional
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

/// Sets up Prometheus metrics exporter
#[cfg(feature = "metrics")]
pub fn setup_metrics_exporter() -> Result<()> {
    let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
    builder.install().map_err(Error::io)
}

/// Sets panic handler to immediately kill the process and its childs with SIGKILL
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

impl DataDeliveryPolicy for () {}
impl DataDeliveryPolicy for usize {}

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
    if is_production() {
        builder.format(|buf, record| writeln!(buf, "{} {}", record.level(), record.args()));
    }
    builder.init();
}

pub mod prelude {
    pub use super::suicide;
    pub use crate::controller::*;
    pub use crate::hub::prelude::*;
    pub use crate::io::prelude::*;
    pub use crate::supervisor::prelude::*;
    pub use crate::time::DurationRT;
    pub use crate::ttlcell::TtlCell;
    pub use bma_ts::{Monotonic, Timestamp};
    pub use roboplc_derive::DataPolicy;
    pub use std::time::Duration;
}
