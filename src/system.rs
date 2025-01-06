use crate::{is_realtime, Result};
use core::fmt;

/// Configure system parameters (global) while the process is running. Does nothing in simulated
/// mode. A wrapper around [`rtsc::system::linux::SystemConfig`] which respects simulated/real-time
/// mode.
///
/// Example:
///
/// ```rust,no_run
/// use roboplc::thread_rt::SystemConfig;
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
