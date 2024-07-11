use crate::{time::Interval, Error, Result};
use bma_ts::{Monotonic, Timestamp};
use colored::Colorize;
use core::fmt;
use libc::cpu_set_t;
use nix::{sys::signal, unistd};
use serde::{Deserialize, Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::BufRead,
    mem,
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, JoinHandle, Scope, ScopedJoinHandle},
    time::Duration,
};
use sysinfo::{Pid, PidExt, ProcessExt, System, SystemExt};
use tracing::warn;

static REALTIME_MODE: AtomicBool = AtomicBool::new(true);

/// The function can be used in test environments to disable real-time functions but keep all
/// methods running with no errors
pub fn set_simulated() {
    REALTIME_MODE.store(false, Ordering::Relaxed);
}

fn is_realtime() -> bool {
    REALTIME_MODE.load(Ordering::Relaxed)
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
pub fn prealloc_heap(size: usize) -> Result<()> {
    if !is_realtime() {
        return Ok(());
    }
    let page_size = unsafe {
        if libc::mallopt(libc::M_MMAP_MAX, 0) != 1 {
            return Err(Error::failed(
                "unable to disable mmap for allocation of large mem regions",
            ));
        }
        if libc::mallopt(libc::M_TRIM_THRESHOLD, -1) != 1 {
            return Err(Error::failed("unable to disable trimming"));
        }
        if libc::mlockall(libc::MCL_FUTURE) == -1 {
            return Err(Error::failed("unable to lock memory pages"));
        };
        usize::try_from(libc::sysconf(libc::_SC_PAGESIZE)).expect("Page size too large")
    };
    let mut heap_mem = vec![0_u8; size];
    std::hint::black_box(move || {
        for i in (0..size).step_by(page_size) {
            heap_mem[i] = 0xff;
        }
    })();
    Ok(())
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
    RoundRobin,
    FIFO,
    Idle,
    Batch,
    DeadLine,
    #[default]
    Other,
}

impl From<Scheduling> for libc::c_int {
    fn from(value: Scheduling) -> Self {
        match value {
            Scheduling::RoundRobin => libc::SCHED_RR,
            Scheduling::FIFO => libc::SCHED_FIFO,
            Scheduling::Idle => libc::SCHED_IDLE,
            Scheduling::Batch => libc::SCHED_BATCH,
            Scheduling::DeadLine => libc::SCHED_DEADLINE,
            Scheduling::Other => libc::SCHED_NORMAL,
        }
    }
}

impl From<libc::c_int> for Scheduling {
    fn from(value: libc::c_int) -> Self {
        match value {
            libc::SCHED_RR => Scheduling::RoundRobin,
            libc::SCHED_FIFO => Scheduling::FIFO,
            libc::SCHED_IDLE => Scheduling::Idle,
            libc::SCHED_BATCH => Scheduling::Batch,
            libc::SCHED_DEADLINE => Scheduling::DeadLine,
            _ => Scheduling::Other,
        }
    }
}

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
    pub fn name(&self) -> &str {
        &self.name
    }
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
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
    pub fn join(self) -> thread::Result<T> {
        self.handle.join()
    }
    pub fn into_join_handle(self) -> JoinHandle<T> {
        self.into()
    }
    /// Returns duration since the task was started
    pub fn elapsed(&self) -> Duration {
        self.info.started_mt.elapsed()
    }
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
    pub fn name(&self) -> &str {
        &self.name
    }
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
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
    pub fn join(self) -> thread::Result<T> {
        self.handle.join()
    }
    pub fn into_join_handle(self) -> ScopedJoinHandle<'scope, T> {
        self.into()
    }
    /// Returns duration since the task was started
    pub fn elapsed(&self) -> Duration {
        self.info.started_mt.elapsed()
    }
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
    pub fn new() -> Self {
        Self::default()
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

fn thread_init_internal(
    tx_tid: oneshot::Sender<(libc::c_int, oneshot::Sender<bool>)>,
    park_on_errors: bool,
) {
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

fn apply_thread_params(tid: libc::c_int, params: &RTParams, quiet: bool) -> Result<()> {
    if !is_realtime() {
        return Ok(());
    }
    if !params.cpu_ids.is_empty() {
        unsafe {
            let mut cpuset: cpu_set_t = mem::zeroed();
            for cpu in &params.cpu_ids {
                libc::CPU_SET(*cpu, &mut cpuset);
            }
            let res = libc::sched_setaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &cpuset);
            if res != 0 {
                if !quiet {
                    eprintln!(
                        "Error setting CPU affinity: {}",
                        std::io::Error::last_os_error()
                    );
                }
                return Err(Error::RTSchedSetAffinity(res));
            }
        }
    }
    if let Some(priority) = params.priority {
        let res = unsafe {
            libc::sched_setscheduler(
                tid,
                params.scheduling.into(),
                &libc::sched_param {
                    sched_priority: priority,
                },
            )
        };
        if res != 0 {
            if !quiet {
                eprintln!(
                    "Error setting scheduler: {}",
                    std::io::Error::last_os_error()
                );
            }
            return Err(Error::RTSchedSetSchduler(res));
        }
    }
    Ok(())
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
    let _ = signal::kill(unistd::Pid::from_raw(pid as i32), signal::Signal::SIGKILL);
}

/// Terminates a process tree with SIGTERM, waits "term_kill_interval" and repeats the opeation
/// with SIGKILL
///
/// If "term_kill_interval" is not set, SIGKILL is used immediately.
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
pub fn kill_pstree(pid: i32, kill_parent: bool, term_kill_interval: Option<Duration>) {
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
/// mode
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
pub struct SystemConfig {
    values: BTreeMap<&'static str, String>,
    prev_values: BTreeMap<&'static str, String>,
}

impl SystemConfig {
    pub fn new() -> Self {
        Self::default()
    }
    /// Set a parameter to configure
    pub fn set<V: fmt::Display>(mut self, key: &'static str, value: V) -> Self {
        self.values.insert(key, value.to_string());
        self
    }
    /// Apply values to /proc/sys keys
    pub fn apply(mut self) -> Result<SystemConfigGuard> {
        if is_realtime() {
            for (key, value) in &self.values {
                let fname = format!("/proc/sys/{}", key);
                let prev_value = fs::read_to_string(&fname)?;
                self.prev_values.insert(key, prev_value);
                fs::write(fname, value)?;
            }
        }
        Ok(SystemConfigGuard { config: self })
    }
}

pub struct SystemConfigGuard {
    config: SystemConfig,
}

impl Drop for SystemConfigGuard {
    fn drop(&mut self) {
        if is_realtime() {
            for (key, value) in &self.config.prev_values {
                if let Err(error) = fs::write(format!("/proc/sys/{}", key), value) {
                    warn!(key, value, %error, "Failed to restore system config");
                }
            }
        }
    }
}

/// Configure CPU governors for the given CPUs
pub struct CpuGovernor {
    prev_governor: BTreeMap<usize, String>,
}

impl CpuGovernor {
    /// Set performance governor for the given CPUs. This sets the maximum frequency for the CPUs,
    /// increasing the power consumption but lowering their latency. It is enough to specify a
    /// single logical core number per physical core. The governor is restored when the returned
    /// guard object is dropped.
    pub fn performance<I>(performance_cpus: I) -> Result<CpuGovernor>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut prev_governor = BTreeMap::new();
        for cpu in performance_cpus {
            let fname = format!(
                "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor",
                cpu
            );
            let prev_value = fs::read_to_string(fname)?;
            prev_governor.insert(cpu, prev_value.trim().to_string());
        }
        for cpu in prev_governor.keys() {
            let fname = format!(
                "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor",
                cpu
            );
            fs::write(fname, "performance")?;
        }
        Ok(CpuGovernor { prev_governor })
    }
}

impl Drop for CpuGovernor {
    fn drop(&mut self) {
        for (cpu, governor) in &self.prev_governor {
            if let Err(error) = fs::write(
                format!(
                    "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor",
                    cpu
                ),
                governor,
            ) {
                warn!(cpu, %error, "Failed to restore governor");
            }
        }
    }
}

/// Get absolute number of CPUs, including isolated
pub fn num_cpus() -> Result<usize> {
    let f = std::fs::File::open("/proc/cpuinfo")?;
    let reader = std::io::BufReader::new(f);
    let lines = reader.lines();
    let mut count = 0;
    for line in lines {
        let line = line?;
        if line
            .split(':')
            .next()
            .ok_or_else(|| Error::failed("invalid line"))?
            .trim_end()
            == "processor"
        {
            count += 1;
        }
    }
    Ok(count)
}
