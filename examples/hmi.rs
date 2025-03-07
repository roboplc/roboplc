use std::sync::atomic;
use std::thread;

use roboplc::controller::prelude::*;
use roboplc::hmi::{self, eframe, egui};
use roboplc::prelude::*;
use rtsc::time::interval;
use tracing::{error, info};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX: usize = 600;

type Message = ();

#[derive(Default)]
struct Variables {
    counter1: atomic::AtomicUsize,
}

// A simple worker which increments the counter every second
#[derive(WorkerOpts)]
#[worker_opts(cpu = 0, priority = 50, scheduling = "fifo", blocking = true)]
struct CounterWorker {}

impl Worker<Message, Variables> for CounterWorker {
    fn run(&mut self, context: &Context<Message, Variables>) -> WResult {
        for _ in interval(Duration::from_secs(1)) {
            if context
                .variables()
                .counter1
                .fetch_add(1, atomic::Ordering::Relaxed)
                == MAX
            {
                context
                    .variables()
                    .counter1
                    .store(0, atomic::Ordering::Relaxed);
            }
            info!("+1");
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    roboplc::setup_panic();
    roboplc::configure_logger(roboplc::LevelFilter::Info);
    if !roboplc::is_production() {
        roboplc::set_simulated();
    }
    roboplc::thread_rt::prealloc_heap(10_000_000)?;
    let mut controller = Controller::<Message, Variables>::new();
    controller.spawn_worker(CounterWorker {})?;
    controller.spawn_worker(HmiWorker {})?;
    controller.register_signals(SHUTDOWN_TIMEOUT)?;
    controller.block_while_online();
    hmi::stop();
    Ok(())
}

// A worker which runs the HMI application
#[derive(WorkerOpts)]
#[worker_opts(cpu = 1, priority = 90, scheduling = "fifo", blocking = true)]
struct HmiWorker {}

impl Worker<Message, Variables> for HmiWorker {
    fn run(&mut self, context: &Context<Message, Variables>) -> WResult {
        // ensure the system is in running state to avoid slowdowns during Weston/Xorg startup
        // not mandatory, as the server startup waits until /run/user/<uid> dir is available
        roboplc::system::wait_running_state()?;
        loop {
            let mut opts = hmi::AppOptions::default();
            if roboplc::is_production() {
                // For production - spawn Weston server
                opts = opts.with_server_options(
                    hmi::ServerKind::Weston
                        .options()
                        .with_kill_delay(Duration::from_secs(2))
                        .with_spawn_delay(Duration::from_secs(5)),
                );
            } else {
                // For development - run windowed app with no server spawned
                opts = opts.windowed();
            }
            if let Err(error) = hmi::run(MyHmiApp {}, context, opts) {
                error!("HMI error: {}", error);
            }
            thread::sleep(Duration::from_secs(5));
        }
    }
}

struct MyHmiApp {}

// The application is basically equal to a typical egui app, the only difference is that the update
// function gets the controller context as an argument.
impl hmi::App for MyHmiApp {
    type M = Message;
    type V = Variables;

    fn update(
        &mut self,
        ctx: &egui::Context,
        _frame: &mut eframe::Frame,
        plc_context: &Context<Self::M, Self::V>,
    ) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "Counter: {}",
                            plc_context
                                .variables()
                                .counter1
                                .load(atomic::Ordering::Relaxed)
                        ))
                        .size(48.0),
                    );
                    ui.horizontal(|ui| {
                        let button_text = egui::RichText::new("RESET")
                            .size(32.0)
                            .strong()
                            .color(egui::Color32::BLACK);

                        let button = egui::Button::new(button_text).fill(egui::Color32::ORANGE);
                        if ui.add(button).clicked() {
                            plc_context
                                .variables()
                                .counter1
                                .store(0, atomic::Ordering::Relaxed);
                            ctx.request_repaint_after(Duration::from_millis(10));
                        }
                        let button_text = egui::RichText::new("SHUTDOWN PLC")
                            .size(32.0)
                            .strong()
                            .color(egui::Color32::BLACK);

                        let button = egui::Button::new(button_text).fill(egui::Color32::LIGHT_RED);
                        if ui.add(button).clicked() {
                            plc_context.terminate();
                        }
                    });
                });
            });
        });
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}
