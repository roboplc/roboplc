use crate::{time::Interval, Error, Result};
use bma_ts::{Monotonic, Timestamp};
use colored::Colorize;
use core::fmt;
#[cfg(target_os = "linux")]
use nix::{sys::signal, unistd};
use serde::{Deserialize, Serialize, Serializer};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, JoinHandle, Scope, ScopedJoinHandle},
    time::Duration,
};
#[cfg(target_os = "linux")]
use sysinfo::PidExt;
use sysinfo::{Pid, ProcessExt, System, SystemExt};

static REALTIME_MODE: AtomicBool = AtomicBool::new(true);

/// The function can be used in test environments to disable real-time functions but keep all
/// methods running with no errors
pub fn set_simulated() {
    REALTIME_MODE.store(false, Ordering::Relaxed);
}

fn is_realtime() -> bool {
    REALTIME_MODE.load(Ordering::Relaxed)
}

#[cfg(not(target_os = "linux"))]
macro_rules! panic_os {
    () => {
        panic!("The function is not supported on this OS");
    };
}

/// The method preallocates a heap memory region with the given size. The method is useful to
/// prevent memory fragmentation and speed up memory allocation. It is highly recommended to call
/// the method at the beginning of the program.
///
/// Does nothing in simulated mode.
///
/// # Panics
///
/// Will panic if the page size is too large (more than usize)
#[allow(unused_variables)]
pub fn prealloc_heap(size: usize) -> Result<()> {
    if !is_realtime() {
        return Ok(());
    }
    rtsc::thread_rt::preallocate_heap(size).map_err(Into::into)
}

/// A thread builder object, similar to [`thread::Builder`] but with real-time capabilities
///
/// Warning: works on Linux systems only
#[derive(Default, Clone)]
pub struct Builder {
    pub(crate) name: Option<String>,
    stack_size: Option<usize>,
    blocking: bool,
    rt_params: RTParams,
    // an internal parameter to suspend (park) failed threads instead of panic
    pub(crate) park_on_errors: bool,
}

/// Thread scheduling policy
///
/// See <https://man7.org/linux/man-pages/man7/sched.7.html>
#[derive(Default, Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Scheduling {
    #[serde(rename = "RR")]
    /// Round-robin
    RoundRobin,
    /// First in, first out
    FIFO,
    /// Idle
    Idle,
    /// Batch
    Batch,
    /// Deadline
    DeadLine,
    #[default]
    /// Other
    Other,
}

impl From<Scheduling> for rtsc::thread_rt::Scheduling {
    fn from(value: Scheduling) -> Self {
        match value {
            Scheduling::RoundRobin => rtsc::thread_rt::Scheduling::RoundRobin,
            Scheduling::FIFO => rtsc::thread_rt::Scheduling::FIFO,
            Scheduling::Idle => rtsc::thread_rt::Scheduling::Idle,
            Scheduling::Batch => rtsc::thread_rt::Scheduling::Batch,
            Scheduling::DeadLine => rtsc::thread_rt::Scheduling::DeadLine,
            Scheduling::Other => rtsc::thread_rt::Scheduling::Other,
        }
    }
}

//#[cfg(target_os = "linux")]
//impl From<Scheduling> for libc::c_int {
//fn from(value: Scheduling) -> Self {
//match value {
//Scheduling::RoundRobin => libc::SCHED_RR,
//Scheduling::FIFO => libc::SCHED_FIFO,
//Scheduling::Idle => libc::SCHED_IDLE,
//Scheduling::Batch => libc::SCHED_BATCH,
//Scheduling::DeadLine => libc::SCHED_DEADLINE,
//Scheduling::Other => libc::SCHED_NORMAL,
//}
//}
//}

//#[cfg(target_os = "linux")]
//impl From<libc::c_int> for Scheduling {
//fn from(value: libc::c_int) -> Self {
//match value {
//libc::SCHED_RR => Scheduling::RoundRobin,
//libc::SCHED_FIFO => Scheduling::FIFO,
//libc::SCHED_IDLE => Scheduling::Idle,
//libc::SCHED_BATCH => Scheduling::Batch,
//libc::SCHED_DEADLINE => Scheduling::DeadLine,
//_ => Scheduling::Other,
//}
//}
//}

macro_rules! impl_builder_from {
    ($t: ty) => {
        impl From<$t> for Builder {
            fn from(s: $t) -> Self {
                Builder::new().name(s)
            }
        }
    };
}

impl_builder_from!(&str);
impl_builder_from!(String);

impl Builder {
    /// Creates a new thread builder
    pub fn new() -> Self {
        Self::default()
    }
    /// The task name SHOULD be 15 characters or less to set a proper thread name
    pub fn name<N: fmt::Display>(mut self, name: N) -> Self {
        self.name = Some(name.to_string());
        self
    }
    /// Overrides the default stack size
    pub fn stack_size(mut self, size: usize) -> Self {
        self.stack_size = Some(size);
        self
    }
    /// A hint for task supervisors that the task blocks the thread (e.g. listens to a socket or
    /// has got a big interval in the main loop, does not return any useful result and should not
    /// be joined)
    ///
    /// For scoped tasks: the task may be still forcibly joined at the end of the scope
    pub fn blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }
    /// Applies real-time parameters to the task
    ///
    /// See [`RTParams`]
    pub fn rt_params(mut self, rt_params: RTParams) -> Self {
        self.rt_params = rt_params;
        self
    }
    fn try_into_thread_builder_name_and_params(
        self,
    ) -> Result<(thread::Builder, String, bool, RTParams, bool)> {
        let mut builder = thread::Builder::new();
        if let Some(ref name) = self.name {
            if name.len() > 15 {
                return Err(Error::invalid_data(format!(
                    "Thread name '{}' is too long (max 15 characters)",
                    name
                )));
            }
            builder = builder.name(name.clone());
        }
        if let Some(stack_size) = self.stack_size {
            builder = builder.stack_size(stack_size);
        }
        Ok((
            builder,
            self.name.unwrap_or_default(),
            self.blocking,
            self.rt_params,
            self.park_on_errors,
        ))
    }
    /// Spawns a task
    ///
    /// # Errors
    ///
    /// Returns errors if the task real-time parameters were set but have been failed to apply. The
    /// task thread is stopped and panicked
    pub fn spawn<F, T>(self, f: F) -> Result<Task<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (builder, name, blocking, rt_params, park_on_errors) =
            self.try_into_thread_builder_name_and_params()?;
        let (tx, rx) = oneshot::channel();
        let handle = builder.spawn(move || {
            thread_init_internal(tx, park_on_errors);
            f()
        })?;
        let tid = thread_init_external(rx, &rt_params, park_on_errors)?;
        Ok(Task {
            name,
            handle,
            blocking,
            tid,
            rt_params,
            info: <_>::default(),
        })
    }
    /// Spawns a periodic task
    ///
    /// # Errors
    ///
    /// Returns errors if the task real-time parameters were set but have been failed to apply. The
    /// task thread is stopped and panicked
    pub fn spawn_periodic<F, T>(self, f: F, mut interval: Interval) -> Result<Task<T>>
    where
        F: Fn() -> T + Send + 'static,
        T: Send + 'static,
    {
        let task_fn = move || loop {
            interval.tick();
            f();
        };
        self.spawn(task_fn)
    }
    /// Spawns a scoped task
    ///
    /// The standard Rust thread [`Scope`] is used.
    ///
    /// # Errors
    ///
    /// Returns errors if the task real-time parameters were set but have been failed to apply. The
    /// task thread is stopped and panicked
    pub fn spawn_scoped<'scope, 'env, F, T>(
        self,
        scope: &'scope Scope<'scope, 'env>,
        f: F,
    ) -> Result<ScopedTask<'scope, T>>
    where
        F: FnOnce() -> T + Send + 'scope,
        T: Send + 'scope,
    {
        let (builder, name, blocking, rt_params, park_on_errors) =
            self.try_into_thread_builder_name_and_params()?;
        let (tx, rx) = oneshot::channel();
        let handle = builder.spawn_scoped(scope, move || {
            thread_init_internal(tx, park_on_errors);
            f()
        })?;
        let tid = thread_init_external(rx, &rt_params, park_on_errors)?;
        Ok(ScopedTask {
            name,
            handle,
            blocking,
            tid,
            rt_params,
            info: <_>::default(),
        })
    }
    /// Spawns a scoped periodic task
    ///
    /// The standard Rust thread [`Scope`] is used.
    ///
    /// # Errors
    ///
    /// Returns errors if the task real-time parameters were set but have been failed to apply. The
    /// task thread is stopped and panicked
    pub fn spawn_scoped_periodic<'scope, 'env, F, T>(
        self,
        scope: &'scope Scope<'scope, 'env>,
        f: F,
        mut interval: Interval,
    ) -> Result<ScopedTask<'scope, T>>
    where
        F: Fn() -> T + Send + 'scope,
        T: Send + 'scope,
    {
        let task_fn = move || loop {
            interval.tick();
            f();
        };
        self.spawn_scoped(scope, task_fn)
    }
}

#[derive(Serialize, Default)]
struct TaskInfo {
    started: Timestamp,
    started_mt: Monotonic,
}

/// An extended task object, returned by [`Builder::spawn()`]
///
/// Can be convered into a standard [`JoinHandle`].
#[derive(Serialize)]
pub struct Task<T> {
    name: String,
    #[serde(
        rename(serialize = "active"),
        serialize_with = "serialize_join_handle_active"
    )]
    handle: JoinHandle<T>,
    blocking: bool,
    tid: libc::c_int,
    rt_params: RTParams,
    info: TaskInfo,
}

impl<T> Task<T> {
    /// Returns the task name
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Returns the task handle
    pub fn handle(&self) -> &JoinHandle<T> {
        &self.handle
    }
    /// Returns current real-time params
    pub fn rt_params(&self) -> &RTParams {
        &self.rt_params
    }
    /// Applies new real-time params
    pub fn apply_rt_params(&mut self, rt_params: RTParams) -> Result<()> {
        if let Err(e) = apply_thread_params(self.tid, &rt_params, false) {
            let _r = apply_thread_params(self.tid, &self.rt_params, false);
            return Err(e);
        }
        self.rt_params = rt_params;
        Ok(())
    }
    /// Returns true if the task is finished
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
    /// Joins the task
    pub fn join(self) -> thread::Result<T> {
        self.handle.join()
    }
    /// Converts the task into a standard [`JoinHandle`]
    pub fn into_join_handle(self) -> JoinHandle<T> {
        self.into()
    }
    /// Returns duration since the task was started
    pub fn elapsed(&self) -> Duration {
        self.info.started_mt.elapsed()
    }
    /// Returns true if the task is blocking
    pub fn is_blocking(&self) -> bool {
        self.blocking
    }
}

impl<T> From<Task<T>> for JoinHandle<T> {
    fn from(task: Task<T>) -> Self {
        task.handle
    }
}

/// An extended task object, returned by [`Builder::spawn_scoped()`]
///
/// Can be convered into a standard [`ScopedJoinHandle`].
#[derive(Serialize)]
pub struct ScopedTask<'scope, T> {
    name: String,
    #[serde(
        rename(serialize = "active"),
        serialize_with = "serialize_scoped_join_handle_active"
    )]
    handle: ScopedJoinHandle<'scope, T>,
    blocking: bool,
    tid: libc::c_int,
    rt_params: RTParams,
    info: TaskInfo,
}

impl<'scope, T> ScopedTask<'scope, T> {
    /// Returns the task name
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Returns the task handle
    pub fn handle(&self) -> &ScopedJoinHandle<T> {
        &self.handle
    }
    /// Returns current real-time params
    pub fn rt_params(&self) -> &RTParams {
        &self.rt_params
    }
    /// Applies new real-time params
    pub fn apply_rt_params(&mut self, rt_params: RTParams) -> Result<()> {
        if let Err(e) = apply_thread_params(self.tid, &rt_params, false) {
            let _r = apply_thread_params(self.tid, &self.rt_params, false);
            return Err(e);
        }
        self.rt_params = rt_params;
        Ok(())
    }
    /// Returns true if the task is finished
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
    /// Joins the task
    pub fn join(self) -> thread::Result<T> {
        self.handle.join()
    }
    /// Converts the task into a standard [`ScopedJoinHandle`]
    pub fn into_join_handle(self) -> ScopedJoinHandle<'scope, T> {
        self.into()
    }
    /// Returns duration since the task was started
    pub fn elapsed(&self) -> Duration {
        self.info.started_mt.elapsed()
    }
    /// Returns true if the task is blocking
    pub fn is_blocking(&self) -> bool {
        self.blocking
    }
}

impl<'scope, T> From<ScopedTask<'scope, T>> for ScopedJoinHandle<'scope, T> {
    fn from(task: ScopedTask<'scope, T>) -> Self {
        task.handle
    }
}

/// Task real-time parameters, used for both regular and scoped tasks
#[derive(Default, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RTParams {
    scheduling: Scheduling,
    priority: Option<libc::c_int>,
    cpu_ids: Vec<usize>,
}

impl RTParams {
    /// Creates a new real-time parameters object
    pub fn new() -> Self {
        Self::default()
    }
    fn as_rtsc_thread_params(&self) -> rtsc::thread_rt::Params {
        rtsc::thread_rt::Params::new()
            .with_priority(self.priority)
            .with_scheduling(self.scheduling.into())
            .with_cpu_ids(&self.cpu_ids)
    }
    /// Sets thread scheduling policy (can be used as build pattern)
    pub fn set_scheduling(mut self, scheduling: Scheduling) -> Self {
        self.scheduling = scheduling;
        if (scheduling == Scheduling::FIFO
            || scheduling == Scheduling::RoundRobin
            || scheduling == Scheduling::DeadLine)
            && self.priority.is_none()
        {
            self.priority = Some(1);
        }
        self
    }
    /// Sets thread priority (can be used as build pattern)
    pub fn set_priority(mut self, priority: libc::c_int) -> Self {
        self.priority = Some(priority);
        self
    }
    /// Sets thread CPU affinity (can be used as build pattern)
    pub fn set_cpu_ids(mut self, ids: &[usize]) -> Self {
        self.cpu_ids = ids.to_vec();
        self
    }
    /// Returns the current scheduling policy
    pub fn scheduling(&self) -> Scheduling {
        self.scheduling
    }
    /// Returns the current thread priority
    pub fn priority(&self) -> Option<i32> {
        self.priority
    }
    /// Returns the current CPU affinity
    pub fn cpu_ids(&self) -> &[usize] {
        &self.cpu_ids
    }
}

#[allow(unused_variables)]
fn thread_init_internal(
    tx_tid: oneshot::Sender<(libc::c_int, oneshot::Sender<bool>)>,
    park_on_errors: bool,
) {
    #[cfg(target_os = "linux")]
    {
        let tid = unsafe { i32::try_from(libc::syscall(libc::SYS_gettid)).unwrap_or(-200) };
        let (tx_ok, rx_ok) = oneshot::channel::<bool>();
        tx_tid.send((tid, tx_ok)).unwrap();
        if !rx_ok.recv().unwrap() {
            if park_on_errors {
                loop {
                    thread::park();
                }
            } else {
                panic!(
                    "THREAD SETUP FAILED FOR `{}`",
                    thread::current().name().unwrap_or_default()
                );
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        panic_os!();
    }
}

#[allow(unused_variables)]
fn thread_init_external(
    rx_tid: oneshot::Receiver<(libc::c_int, oneshot::Sender<bool>)>,
    params: &RTParams,
    quiet: bool,
) -> Result<libc::c_int> {
    let (tid, tx_ok) = rx_tid.recv()?;
    if tid < 0 {
        tx_ok.send(false).map_err(|e| Error::Comm(e.to_string()))?;
        return Err(Error::RTGetTId(tid));
    }
    if let Err(e) = apply_thread_params(tid, params, quiet) {
        tx_ok.send(false).map_err(|e| Error::Comm(e.to_string()))?;
        return Err(e);
    }
    tx_ok.send(true).map_err(|e| Error::Comm(e.to_string()))?;
    Ok(tid)
}

#[allow(unused_variables)]
fn apply_thread_params(tid: libc::c_int, params: &RTParams, quiet: bool) -> Result<()> {
    if !is_realtime() {
        return Ok(());
    }
    rtsc::thread_rt::apply(tid, &params.as_rtsc_thread_params()).map_err(Into::into)
}

macro_rules! impl_serialize_join_handle {
    ($fn_name:ident, $handle_type:ty) => {
        fn $fn_name<T, S>(
            handle: &$handle_type,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_bool(!handle.is_finished())
        }
    };
}

impl_serialize_join_handle!(serialize_join_handle_active, JoinHandle<T>);
impl_serialize_join_handle!(serialize_scoped_join_handle_active, ScopedJoinHandle<T>);

#[allow(clippy::cast_possible_wrap)]
pub(crate) fn suicide_myself(delay: Duration, warn: bool) {
    let pid = std::process::id();
    thread::sleep(delay);
    if warn {
        eprintln!("{}", "KILLING THE PROCESS".red().bold());
    }
    kill_pstree(pid as i32, false, None);
    #[cfg(target_os = "linux")]
    let _ = signal::kill(unistd::Pid::from_raw(pid as i32), signal::Signal::SIGKILL);
    #[cfg(not(target_os = "linux"))]
    {
        panic_os!();
    }
}

/// Terminates a process tree with SIGTERM, waits "term_kill_interval" and repeats the opeation
/// with SIGKILL
///
/// If "term_kill_interval" is not set, SIGKILL is used immediately.
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss, unused_variables)]
pub fn kill_pstree(pid: i32, kill_parent: bool, term_kill_interval: Option<Duration>) {
    #[cfg(target_os = "linux")]
    {
        let mut sys = System::new();
        sys.refresh_processes();
        let mut pids = BTreeSet::new();
        if let Some(delay) = term_kill_interval {
            kill_process_tree(
                Pid::from_u32(pid as u32),
                &mut sys,
                &mut pids,
                signal::Signal::SIGTERM,
                kill_parent,
            );
            thread::sleep(delay);
            sys.refresh_processes();
        }
        kill_process_tree(
            Pid::from_u32(pid as u32),
            &mut sys,
            &mut pids,
            signal::Signal::SIGTERM,
            kill_parent,
        );
    }
    #[cfg(not(target_os = "linux"))]
    {
        panic_os!();
    }
}

#[cfg(target_os = "linux")]
fn kill_process_tree(
    pid: Pid,
    sys: &mut sysinfo::System,
    pids: &mut BTreeSet<Pid>,
    signal: nix::sys::signal::Signal,
    kill_parent: bool,
) {
    sys.refresh_processes();
    if kill_parent {
        pids.insert(pid);
    }
    get_child_pids_recursive(pid, sys, pids);
    for cpid in pids.iter() {
        #[allow(clippy::cast_possible_wrap)]
        let _ = signal::kill(unistd::Pid::from_raw(cpid.as_u32() as i32), signal);
    }
}

#[allow(dead_code)]
fn get_child_pids_recursive(pid: Pid, sys: &System, to: &mut BTreeSet<Pid>) {
    for (i, p) in sys.processes() {
        if let Some(parent) = p.parent() {
            if parent == pid {
                to.insert(*i);
                get_child_pids_recursive(*i, sys, to);
            }
        };
    }
}

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
