use crate::{is_realtime, Result};
use core::fmt;
use std::convert::Infallible;
use std::str::FromStr;

/// Configure system parameters (global) while the process is running. Does nothing in simulated
/// mode. A wrapper around [`rtsc::system::linux::SystemConfig`] which respects simulated/real-time
/// mode.
///
/// Example:
///
/// ```rust,no_run
/// use roboplc::system::SystemConfig;
///
/// let _sys = SystemConfig::new().set("kernel/sched_rt_runtime_us", -1)
///     .apply()
///     .expect("Unable to set system config");
/// // some code
/// // system config is restored at the end of the scope
/// ```
#[allow(clippy::module_name_repetitions)]
#[derive(Default)]
pub struct SystemConfig(rtsc::system::linux::SystemConfig);

impl SystemConfig {
    /// Creates a new system config object
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Set a parameter to configure
    pub fn set<V: fmt::Display>(mut self, key: &'static str, value: V) -> Self {
        if is_realtime() {
            self.0 = self.0.set(key, value);
        }
        self
    }
    /// Apply values to /proc/sys keys
    pub fn apply(self) -> Result<rtsc::system::linux::SystemConfigGuard> {
        if is_realtime() {
            return self.0.apply().map_err(Into::into);
        }
        Ok(rtsc::system::linux::SystemConfigGuard::default())
    }
}

/// Configure CPU governors for the given CPUs. A wrapper around
/// [`rtsc::system::linux::CpuGovernor`] which respects simulated/real-time mode.
pub struct CpuGovernor(#[allow(dead_code)] rtsc::system::linux::CpuGovernor);

impl CpuGovernor {
    /// Set performance governor for the given CPUs. This sets the maximum frequency for the CPUs,
    /// increasing the power consumption but lowering their latency. It is enough to specify a
    /// single logical core number per physical core. The governor is restored when the returned
    /// guard object is dropped.
    pub fn performance<I>(performance_cpus: I) -> Result<CpuGovernor>
    where
        I: IntoIterator<Item = usize>,
    {
        if is_realtime() {
            let inner = rtsc::system::linux::CpuGovernor::performance(performance_cpus)?;
            Ok(Self(inner))
        } else {
            Ok(Self(rtsc::system::linux::CpuGovernor::default()))
        }
    }
}

/// Standard systemd system state variants.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StateVariant {
    /// The system is initializing.
    Initializing,
    /// The system is starting.
    Starting,
    /// The system is running.
    Running,
    /// The system is degraded.
    Degraded,
    /// The system is in maintenance mode.
    Maintenance,
    /// The system is stopping.
    Stopping,
    /// The system is in some other state.
    Other,
}

impl fmt::Display for StateVariant {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StateVariant::Initializing => write!(f, "initializing"),
            StateVariant::Starting => write!(f, "starting"),
            StateVariant::Running => write!(f, "running"),
            StateVariant::Degraded => write!(f, "degraded"),
            StateVariant::Maintenance => write!(f, "maintenance"),
            StateVariant::Stopping => write!(f, "stopping"),
            StateVariant::Other => write!(f, "other"),
        }
    }
}

impl FromStr for StateVariant {
    type Err = Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "initializing" => StateVariant::Initializing,
            "starting" => StateVariant::Starting,
            "running" => StateVariant::Running,
            "degraded" => StateVariant::Degraded,
            "maintenance" => StateVariant::Maintenance,
            "stopping" => StateVariant::Stopping,
            _ => StateVariant::Other,
        })
    }
}

/// Get the current system state. Use CLI instead of direct D-Bus calls to avoid unnecessary
/// dependencies.
pub fn state() -> Result<StateVariant> {
    std::process::Command::new("systemctl")
        .arg("is-system-running")
        .output()
        .map_err(Into::into)
        .and_then(|output| {
            let state = std::str::from_utf8(&output.stdout).unwrap_or_default();
            state.trim().parse().map_err(Into::into)
        })
}

/// Wait until the system is in the running state.
pub fn wait_running_state() -> Result<()> {
    loop {
        if state()? == StateVariant::Running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Ok(())
}

// A variant with D-Bus for future reference
/*
let connection = Connection::new_system()?;
let proxy = connection.with_proxy(
    "org.freedesktop.systemd1",
    "/org/freedesktop/systemd1",
    Duration::from_millis(5000),
);
let (state_variant,): (Variant<String>,) = proxy.method_call(
    "org.freedesktop.DBus.Properties",
    "Get",
    ("org.freedesktop.systemd1.Manager", "SystemState"),
)?;
state_variant.0.parse::<SystemStateVariant>()?;
*/
