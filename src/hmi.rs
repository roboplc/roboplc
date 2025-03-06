use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::Path,
    process::Child,
    thread,
    time::Duration,
};

use crate::locking::Mutex;
use crate::{prelude::Context, DataDeliveryPolicy};
use crate::{Error, Result};
use eframe::EventLoopBuilderHook;
use once_cell::sync::Lazy;
use tracing::{error, warn};

pub use eframe;
pub use egui;

static SERVER_INSTANCE: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));

/// Graphics server options
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerOptions {
    command: OsString,
    kill_command: Option<OsString>,
    env: BTreeMap<String, String>,
    wait_for: Option<OsString>,
    kill_delay: Duration,
    spawn_delay: Duration,
}

impl ServerOptions {
    /// Creates a new server options with the given launch command
    pub fn new<C: AsRef<OsStr>>(command: C) -> Self {
        Self {
            command: command.as_ref().to_owned(),
            kill_command: None,
            env: <_>::default(),
            wait_for: None,
            spawn_delay: Duration::from_secs(5),
            kill_delay: Duration::from_secs(5),
        }
    }
    /// The command is executed to terminate the previous server instance if there is a conflict
    /// (e.g. the previous program instance crashed and left the server running).
    pub fn with_terminate_previous_command<C: AsRef<OsStr>>(mut self, kill_command: C) -> Self {
        self.kill_command = Some(kill_command.as_ref().to_owned());
        self
    }
    /// Adds an environment variable to the HMI thread after the server is started
    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }
    /// Wait for a file (server socket) before starting the application
    pub fn with_wait_for<C: AsRef<OsStr>>(mut self, wait_for: C) -> Self {
        self.wait_for = Some(wait_for.as_ref().to_owned());
        self
    }
    /// Delay before starting the application after the server is started
    pub fn with_spawn_delay(mut self, delay: Duration) -> Self {
        self.spawn_delay = delay;
        self
    }
    /// Delay after the server is killed to ensure that TTY is released
    pub fn with_kill_delay(mut self, delay: Duration) -> Self {
        self.kill_delay = delay;
        self
    }
}

/// Graphics server kind
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ServerKind {
    /// Weston server
    Weston,
    /// Legacy weston server, adds `--tty=1` to the command line
    WestonLegacy,
    /// Xorg server
    Xorg,
}

impl ServerKind {
    /// Returns the server options for the given server kind
    pub fn options(self: ServerKind) -> ServerOptions {
        match self {
            ServerKind::Weston | ServerKind::WestonLegacy => {
                let mut opts = if self == ServerKind::Weston {
                    ServerOptions::new("weston")
                } else {
                    ServerOptions::new("weston --tty=1")
                };
                opts = opts
                    .with_env("WAYLAND_DISPLAY", "wayland-1")
                    .with_wait_for("/run/user/0/wayland-1")
                    .with_terminate_previous_command("pkill -KILL weston");
                opts
            }
            ServerKind::Xorg => {
                let mut opts = ServerOptions::new("Xorg :0");
                opts = opts
                    .with_env("DISPLAY", ":0")
                    .with_wait_for("/tmp/.X11-unix/X0")
                    .with_terminate_previous_command("pkill -KILL Xorg");
                opts
            }
        }
    }
}

/// HMI application options
#[derive(Clone, Debug)]
pub struct AppOptions {
    fullscreen: bool,
    title: String,
    dimensions: Option<(u16, u16)>,
    server_options: Option<ServerOptions>,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            fullscreen: true,
            title: "HMI".to_string(),
            dimensions: None,
            server_options: None,
        }
    }
}

impl AppOptions {
    /// Creates a new HMI application options
    pub fn new() -> Self {
        Self::default()
    }
    /// Runs the HMI application in windowed mode (default is fullscreen)
    pub fn windowed(mut self) -> Self {
        self.fullscreen = false;
        self
    }
    /// Sets the title of the HMI application window (required for Xorg)
    pub fn with_dimensions(mut self, width: u16, height: u16) -> Self {
        self.dimensions = Some((width, height));
        self
    }
    /// Sets the server options
    pub fn with_server_options(mut self, opts: ServerOptions) -> Self {
        self.server_options = Some(opts);
        self
    }
}

/// HMI application, a wrapper around an eframe application
pub trait App {
    /// Context message
    type M: DataDeliveryPolicy + Send + Sync + Clone;
    /// Context variables
    type V: Send;
    /// UI update, similar to eframe::App::update but with PLC program context
    fn update(
        &mut self,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
        plc_context: &Context<Self::M, Self::V>,
    );
}

/// Stop HMI server if running
pub fn stop() {
    if let Some(child) = SERVER_INSTANCE.lock().take() {
        let pid = child.id();
        #[allow(clippy::cast_possible_wrap)]
        crate::thread_rt::kill_pstree(pid as i32, true, None);
    }
}

/// Start HMI server (for own use, not required for the HMI application)
pub fn start_server(server_options: ServerOptions) {
    if let Some(kill_command) = &server_options.kill_command {
        match std::process::Command::new("sh")
            .args([OsString::from("-c"), kill_command.to_owned()])
            .spawn()
        {
            Ok(mut child) => {
                let _ = child.wait();
                thread::sleep(server_options.kill_delay);
            }
            Err(error) => {
                warn!(?error, "Failed to terminate previous server instance");
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let uid = unsafe { libc::getuid() };
        if let Err(error) = std::fs::create_dir_all(Path::new("/run/user").join(uid.to_string())) {
            error!(?error, "Failed to create /run/user/<uid> directory");
        }
    }
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/0");
    let child = match std::process::Command::new("sh")
        .args([OsString::from("-c"), server_options.command.clone()])
        .spawn()
    {
        Ok(c) => c,
        Err(error) => {
            error!(?error, "Failed to start graphics server");
            loop {
                thread::park();
            }
        }
    };
    *SERVER_INSTANCE.lock() = Some(child);
    for (key, value) in &server_options.env {
        std::env::set_var(key, value);
    }
    if let Some(wait_for) = server_options.wait_for {
        let wait_for = Path::new(&wait_for);
        loop {
            if wait_for.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    thread::sleep(server_options.spawn_delay);
}

/// Run HMI application.
///
/// Starts the HMI server if required, then runs the HMI application.
pub fn run<A, M, V>(app: A, plc_context: &Context<M, V>, options: AppOptions) -> Result<()>
where
    A: App<M = M, V = V>,
    M: DataDeliveryPolicy + Send + Sync + Clone + 'static,
    V: Send,
{
    stop();
    if let Some(opts) = options.server_options {
        start_server(opts);
    };
    let event_loop_builder: Option<EventLoopBuilderHook> = Some(Box::new(|event_loop_builder| {
        winit::platform::wayland::EventLoopBuilderExtWayland::with_any_thread(
            event_loop_builder,
            true,
        );
    }));
    let mut viewport = egui::ViewportBuilder::default().with_fullscreen(options.fullscreen);
    if let Some((width, height)) = options.dimensions {
        viewport = viewport.with_inner_size((f32::from(width), f32::from(height)));
    }
    let e_options = eframe::NativeOptions {
        viewport,
        event_loop_builder,
        ..Default::default()
    };
    let plc_context = plc_context.clone();
    eframe::run_native(
        &options.title,
        e_options,
        Box::new(|_cc| Ok(Box::new(Hmi { app, plc_context }))),
    )
    .map_err(Error::failed)
}

struct Hmi<A, M, V>
where
    A: App<M = M, V = V>,
    M: DataDeliveryPolicy + Send + Sync + Clone + 'static,
    V: Send,
{
    app: A,
    plc_context: Context<M, V>,
}

impl<A, M, V> eframe::App for Hmi<A, M, V>
where
    A: App<M = M, V = V>,
    M: DataDeliveryPolicy + Send + Sync + Clone + 'static,
    V: Send,
{
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.update(ctx, _frame, &self.plc_context);
    }
}
