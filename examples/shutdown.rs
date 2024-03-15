// The example provides a graceful shutdown of the controller.

use roboplc::{prelude::*, time::interval};
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use tracing::info;

#[derive(DataPolicy, Clone)]
enum Message {
    Data(u8),
    // Terminate signal for workers which listen on hub events
    Terminate,
}

#[derive(WorkerOpts)]
#[worker_opts(name = "parser")]
struct DataParser {}

impl Worker<Message, ()> for DataParser {
    fn run(&mut self, context: &Context<Message, ()>) -> WResult {
        let hc = context.hub().register(
            self.worker_name(),
            event_matches!(Message::Data(_) | Message::Terminate),
        )?;
        for msg in hc {
            match msg {
                Message::Data(data) => {
                    info!(worker = self.worker_name(), data = data);
                }
                // This worker terminates itself when it receives the Terminate message
                Message::Terminate => {
                    break;
                }
            }
        }
        Ok(())
    }
}

#[derive(WorkerOpts)]
#[worker_opts(name = "generator")]
struct DataGenerator {}

impl Worker<Message, ()> for DataGenerator {
    fn run(&mut self, context: &Context<Message, ()>) -> WResult {
        for _ in interval(Duration::from_secs(1)) {
            context.hub().send(Message::Data(42));
            // This worker terminates itself when the controller goes to the stopping state
            if !context.is_online() {
                break;
            }
        }
        Ok(())
    }
}

#[derive(WorkerOpts)]
#[worker_opts(name = "sighandle")]
struct SignalHandler {}

impl Worker<Message, ()> for SignalHandler {
    // this worker listens to SIGINT and SIGTERM signals, sends a Terminate message to the hub and
    // sets the controller state to Stopping
    fn run(&mut self, context: &Context<Message, ()>) -> WResult {
        let mut signals = Signals::new([SIGTERM, SIGINT])?;

        if let Some(sig) = signals.forever().next() {
            match sig {
                SIGTERM | SIGINT => {
                    info!("terminating");
                    // it is really important to set max shutdown timeout for the controller if the
                    // controller does not terminate in the given time, the process and all its
                    // sub-processes are forcibly killed
                    suicide(Duration::from_secs(2), true);
                    // set controller state to terminating
                    context.set_state(ControllerStateKind::Stopping);
                    // send Terminate message to workers who listen to the hub
                    context.hub().send(Message::Terminate);
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    let mut controller = Controller::<Message, ()>::new();
    controller.spawn_worker(DataGenerator {})?;
    controller.spawn_worker(DataParser {})?;
    controller.spawn_worker(SignalHandler {})?;
    info!("controller started");
    controller.block();
    info!("controller terminated");
    Ok(())
}
