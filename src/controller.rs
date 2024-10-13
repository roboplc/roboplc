use std::{
    sync::{
        atomic::{AtomicI8, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

#[cfg(target_os = "linux")]
use crate::suicide;
use crate::{
    critical,
    hub::Hub,
    supervisor::Supervisor,
    thread_rt::{Builder, RTParams, Scheduling},
    Error, Result,
};
pub use roboplc_derive::WorkerOpts;
use rtsc::data_policy::DataDeliveryPolicy;
#[cfg(target_os = "linux")]
use signal_hook::{
    consts::{SIGINT, SIGTERM, SIGUSR2},
    iterator::Signals,
};
use tracing::error;

/// Controller prelude
pub mod prelude {
    pub use super::{Context, Controller, WResult, Worker, WorkerOptions};
    pub use roboplc_derive::WorkerOpts;
}

/// Result type, which must be returned by workers' `run` method
pub type WResult = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Sleep step (used in blocking)
pub const SLEEP_STEP: Duration = Duration::from_millis(100);

/// Controller state beacon. Can be cloned and shared with no limitations.
#[derive(Clone)]
pub struct State {
    state: Arc<AtomicI8>,
}

impl State {
    fn new() -> Self {
        Self {
            state: AtomicI8::new(ControllerStateKind::Starting as i8).into(),
        }
    }
    /// Set controller state
    pub fn set(&self, state: ControllerStateKind) {
        self.state.store(state as i8, Ordering::SeqCst);
    }
    /// Get controller state
    pub fn get(&self) -> ControllerStateKind {
        ControllerStateKind::from(self.state.load(Ordering::SeqCst))
    }
    /// Is the controller online (starting or running)
    pub fn is_online(&self) -> bool {
        self.get() >= ControllerStateKind::Starting
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// Controller state kind
#[derive(Default, Eq, PartialEq, Clone, Copy, Ord, PartialOrd)]
#[repr(i8)]
#[allow(clippy::module_name_repetitions)]
pub enum ControllerStateKind {
    #[default]
    /// The controller is starting
    Starting = 0,
    /// The controller is active (accepting tasks)
    Active = 1,
    /// The controller is running (tasks are being executed)
    Running = 2,
    /// The controller is stopping
    Stopping = -1,
    /// The controller is stopped
    Stopped = -100,
    /// The controller state is unknown
    Unknown = -128,
}

impl From<i8> for ControllerStateKind {
    fn from(v: i8) -> Self {
        match v {
            0 => ControllerStateKind::Starting,
            1 => ControllerStateKind::Active,
            2 => ControllerStateKind::Running,
            -100 => ControllerStateKind::Stopped,
            _ => ControllerStateKind::Unknown,
        }
    }
}

/// Controller, used to manage workers and their context
///
/// Generic parameter `D` is the message type for the controller's [`Hub`] messages.
/// Generic parameter `V` is the type of shared variables. If shared variables are not required, it
/// can be set to `()`.
///
pub struct Controller<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    supervisor: Supervisor<()>,
    hub: Hub<D>,
    state: State,
    variables: Arc<V>,
}

impl<D, V> Controller<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    /// Creates a new controller instance, variables MUST implement [`Default`] trait
    pub fn new() -> Self
    where
        V: Default,
    {
        Self {
            supervisor: <_>::default(),
            hub: <_>::default(),
            state: State::new(),
            variables: <_>::default(),
        }
    }
    /// Creates a new controller instance with a pre-defined variables object
    pub fn new_with_variables(variables: V) -> Self {
        Self {
            supervisor: <_>::default(),
            hub: <_>::default(),
            state: State::new(),
            variables: Arc::new(variables),
        }
    }
    /// Spawns a worker
    pub fn spawn_worker<W: Worker<D, V> + WorkerOptions + 'static>(
        &mut self,
        mut worker: W,
    ) -> Result<()> {
        let context = self.context();
        let mut rt_params = RTParams::new().set_scheduling(worker.worker_scheduling());
        if let Some(priority) = worker.worker_priority() {
            rt_params = rt_params.set_priority(priority);
        }
        if let Some(cpu_ids) = worker.worker_cpu_ids() {
            rt_params = rt_params.set_cpu_ids(cpu_ids);
        }
        let mut builder = Builder::new()
            .name(worker.worker_name())
            .rt_params(rt_params)
            .blocking(worker.worker_is_blocking());
        if let Some(stack_size) = worker.worker_stack_size() {
            builder = builder.stack_size(stack_size);
        }
        self.supervisor.spawn(builder, move || {
            if let Err(e) = worker.run(&context) {
                error!(worker=worker.worker_name(), error=%e, "worker terminated");
                critical(&format!(
                    "Worker {} terminated: {}",
                    worker.worker_name(),
                    e
                ));
            }
        })?;
        Ok(())
    }
    /// Spawns a task thread (non-real-time) with the default options
    pub fn spawn_task<F>(&mut self, name: &str, f: F) -> Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        self.supervisor.spawn(Builder::new().name(name), f)?;
        Ok(())
    }
    /// Registers SIGINT, SIGTERM and SIGUSR2 signals to a thread which terminates the controller
    /// with dummy handlers (see [`Controller::register_signals_with_handlers()`]).
    pub fn register_signals(&mut self, shutdown_timeout: Duration) -> Result<()> {
        self.register_signals_with_handlers(|_| {}, |_| Ok(()), shutdown_timeout)
    }
    /// Registers SIGINT and SIGTERM signals to a thread which terminates the controller.
    ///     
    /// Note: to properly terminate all workers must either periodically check the controller state
    /// with [`Context::is_online()`] or be marked as blocking by overriding
    /// [`WorkerOptions::worker_is_blocking()`] (or setting `blocking` to `true` in [`WorkerOpts`]
    /// derive macro).
    ///
    /// Workers that listen to hub messages may also receive a custom termination message and gracefully
    /// shut themselves down. For such functionality a custom signal handler should be implemented
    /// (See <https://github.com/roboplc/roboplc/blob/main/examples/shutdown.rs>).
    ///
    /// The thread is automatically spawned with FIFO scheduling and the highest priority on CPU 0
    /// or falled back to non-realtime.
    ///
    /// If the reload handler function returns error, the reload process is aborted. Otherwise, the
    /// executable is reloaded (see [`crate::reload_executable()`]).
    pub fn register_signals_with_handlers<SH, RH>(
        &mut self,
        #[allow(unused_variables)] shutdown_handler_fn: SH,
        #[allow(unused_variables)] reload_handler_fn: RH,
        #[allow(unused_variables)] shutdown_timeout: Duration,
    ) -> Result<()>
    where
        SH: Fn(&Context<D, V>) + Send + Sync + 'static,
        RH: Fn(&Context<D, V>) -> Result<()> + Send + Sync + 'static,
    {
        let shutdown_handler = Arc::new(shutdown_handler_fn);
        let reload_handler = Arc::new(reload_handler_fn);
        let mut builder = Builder::new().name("RoboPLCSigRT").rt_params(
            RTParams::new()
                .set_priority(99)
                .set_scheduling(Scheduling::FIFO)
                .set_cpu_ids(&[0]),
        );
        builder.park_on_errors = true;
        macro_rules! sig_handler {
            ($shutdown_handler: expr, $reload_handler: expr) => {{
                #[cfg(target_os = "linux")]
                {
                    let context = self.context();
                    let mut signals = Signals::new([SIGTERM, SIGINT, SIGUSR2])?;
                    move || {
                        if let Some(sig) = signals.forever().next() {
                            match sig {
                                SIGTERM | SIGINT => {
                                    suicide(shutdown_timeout, true);
                                    $shutdown_handler(&context);
                                    context.terminate();
                                }
                                SIGUSR2 => {
                                    tracing::warn!("Performing live reload");
                                    if let Err(e) = $reload_handler(&context) {
                                        error!(error=%e, "reload handler");
                                    } else if let Err(e) = crate::reload_executable() {
                                        panic!("Live reload failed: {}", e);
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    move || {}
                }
            }};
        }
        #[allow(unused_variables)]
        let sh = shutdown_handler.clone();
        #[allow(unused_variables)]
        let rh = reload_handler.clone();
        if let Err(e) = self.supervisor.spawn(builder.clone(), sig_handler!(sh, rh)) {
            if !matches!(e, Error::RTSchedSetSchduler(_)) {
                return Err(e);
            }
        } else {
            return Ok(());
        }
        // fall-back to non-rt handler
        let builder = builder.name("RoboPLCSig").rt_params(RTParams::new());
        self.supervisor
            .spawn(builder, sig_handler!(shutdown_handler, reload_handler))?;
        Ok(())
    }
    fn context(&self) -> Context<D, V> {
        Context {
            hub: self.hub.clone(),
            state: self.state.clone(),
            variables: self.variables.clone(),
        }
    }
    /// Blocks until all tasks/workers are finished
    pub fn block(&mut self) {
        self.supervisor.join_all();
        self.state.set(ControllerStateKind::Stopped);
    }
    /// Blocks until the controller goes into stopping/stopped
    pub fn block_while_online(&self) {
        while self.state.is_online() {
            thread::sleep(SLEEP_STEP);
        }
        self.state.set(ControllerStateKind::Stopped);
    }
    /// Is the controller online (starting or running)
    pub fn is_online(&self) {
        self.state.is_online();
    }
    /// Sets controller state to Stopping
    pub fn terminate(&mut self) {
        self.state.set(ControllerStateKind::Stopping);
    }
    /// State beacon
    pub fn state(&self) -> &State {
        &self.state
    }
    /// Controller [`Hub`] instance
    pub fn hub(&self) -> &Hub<D> {
        &self.hub
    }
    /// Controller [`Supervisor`] instance
    pub fn supervisor(&self) -> &Supervisor<()> {
        &self.supervisor
    }
    /// Controller shared variables
    pub fn variables(&self) -> &Arc<V> {
        &self.variables
    }
}

impl<D, V> Default for Controller<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

/// The context type is used to give workers access to the controller's hub, state, and shared
/// variables.
pub struct Context<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    hub: Hub<D>,
    state: State,
    variables: Arc<V>,
}

impl<D, V> Clone for Context<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    fn clone(&self) -> Self {
        Self {
            hub: self.hub.clone(),
            state: self.state.clone(),
            variables: self.variables.clone(),
        }
    }
}

impl<D, V> Context<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    /// Controller's hub instance
    pub fn hub(&self) -> &Hub<D> {
        &self.hub
    }
    /// Controller's shared variables
    pub fn variables(&self) -> &Arc<V> {
        &self.variables
    }
    /// Controller's state
    pub fn get_state(&self) -> ControllerStateKind {
        self.state.get()
    }
    /// Set controller's state
    pub fn set_state(&self, state: ControllerStateKind) {
        self.state.set(state);
    }
    /// Is the controller online (starting or running)
    pub fn is_online(&self) -> bool {
        self.state.is_online()
    }
    /// Sets controller state to Stopping
    pub fn terminate(&self) {
        self.state.set(ControllerStateKind::Stopping);
    }
}

/// The trait which MUST be implemented by all workers
pub trait Worker<D: DataDeliveryPolicy + Clone + Send + Sync + 'static, V: Send>:
    Send + Sync
{
    /// The worker's main function, started by [`Controller::spawn_worker()`]. If the function
    /// returns an error, the process is terminated using [`critical()`].
    fn run(&mut self, context: &Context<D, V>) -> WResult;
}

/// The trait which MUST be implemented by all workers
pub trait WorkerOptions {
    /// A mandatory method, an unique name for the worker
    fn worker_name(&self) -> &str;
    /// The stack size for the worker thread
    fn worker_stack_size(&self) -> Option<usize> {
        None
    }
    /// The [`Scheduling`] policy for the worker thread
    fn worker_scheduling(&self) -> Scheduling {
        Scheduling::default()
    }
    /// The scheduled priority for the worker thread
    fn worker_priority(&self) -> Option<i32> {
        None
    }
    /// The CPU ID(s) affinity for the worker thread
    fn worker_cpu_ids(&self) -> Option<&[usize]> {
        None
    }
    /// A hint for task supervisors that the worker blocks the thread (e.g. listens to a socket or
    /// has got a big interval in the main loop, does not return any useful result and should not
    /// be joined)
    fn worker_is_blocking(&self) -> bool {
        false
    }
}
