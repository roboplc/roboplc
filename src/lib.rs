#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]
use std::{mem, sync::Arc, time::Duration};

use thread_rt::{RTParams, Scheduling};

/// Event buffers
pub mod buf;
/// In-process data communication pub/sub hub
pub mod hub;
/// Policy-based channels, synchronous edition
pub mod pchannel;
/// Policy-based channels, asynchronous edition
pub mod pchannel_async;
/// Policy-based data storages
pub mod pdeque;
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
    #[error("hub send error {0}")]
    HubSend(Box<Error>),
    #[error("hub client already registered: {0}")]
    HubAlreadyRegistered(Arc<str>),
    #[error("I/O error {0}")]
    IO(String),
    #[error("RT SYS_gettid {0}")]
    RTGetTId(libc::c_int),
    #[error("RT sched_setaffinity {0}")]
    RTSchedSetAffinity(libc::c_int),
    #[error("RT sched_setscheduler {0}")]
    RTSchedSetSchduler(libc::c_int),
    #[error("Task name must be specified when spawning by a supervisor")]
    SupervisorNameNotSpecified,
    #[error("Task already registered")]
    SupervisorDuplicateTask,
    #[error("Task not found")]
    SupervisorTaskNotFound,
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
impl_error!(oneshot::RecvError, IO);

impl Error {
    pub fn is_skipped(&self) -> bool {
        matches!(self, Error::ChannelSkipped)
    }
}

/// Data delivery policies, used by [`hub::Hub`], [`pchannel::Receiver`] and [`pdeque::Deque`]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum DeliveryPolicy {
    #[default]
    /// always deliver, fail if no room (default)
    Always,
    /// skip delivery if no room
    Optional,
    /// always deliver the frame but always in a single copy (latest)
    Single,
    /// deliver a single latest copy, skip if no room
    SingleOptional,
}

/// Implements delivery policies for own data types
pub trait DataDeliveryPolicy
where
    Self: Sized,
{
    /// Delivery policy
    fn delivery_policy(&self) -> DeliveryPolicy {
        DeliveryPolicy::Always
    }
    /// Priority, for ordered
    fn priority(&self) -> usize {
        0
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

/// Terminates the current process and all its subprocesses in the specified period of time with
/// SIGKILL command. Useful if a process is unable to shut it down gracefully within a specified
/// period of time.
///
/// Prints warnings to STDOUT if warn is true
pub fn suicide(delay: Duration, warn: bool) {
    let mut builder = thread_rt::Builder::new().name("suicide").rt_params(
        RTParams::new()
            .set_priority(99)
            .set_scheduling(Scheduling::FIFO),
    );
    builder.park_on_errors = true;
    let res = builder.spawn(move || {
        dbg!("realtime");
        thread_rt::suicide_myself(delay, warn);
    });
    if res.is_err() {
        std::thread::spawn(move || {
            thread_rt::suicide_myself(delay, warn);
        });
    };
}
