// The example provides a graceful shutdown of the controller using a custom signal handler.

use roboplc::{prelude::*, time::interval};
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use tracing::info;

// The maximum shutdown time
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(DataPolicy, Clone)]
enum Message {
    Data(u8),
    // Terminate signal for workers which listen on hub events
    Terminate,
}

/// A worker which has got a blocking loop (e.g. listening to a socket, having a long cycle etc.)
/// and it is not possible to terminate it immediately. In this case the worker is not joined on
/// shutdown
#[derive(WorkerOpts)]
#[worker_opts(name = "veryblocking", blocking = true)]
struct VeryBlocking {}

impl Worker<Message, ()> for VeryBlocking {
    fn run(&mut self, _context: &Context<Message, ()>) -> WResult {
        for _ in interval(Duration::from_secs(120)) {
            info!(worker = self.worker_name(), "I am still running");
        }
        Ok(())
    }
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
        for _ in interval(Duration::from_secs(1)).take_while(|_| context.is_online()) {
            context.hub().send(Message::Data(42));
            // This worker terminates itself when the controller goes to the stopping state
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
                    suicide(SHUTDOWN_TIMEOUT, true);
                    // set controller state to Stopping
                    context.terminate();
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
    roboplc::setup_panic();
    roboplc::configure_logger(roboplc::LevelFilter::Info);
    let mut controller = Controller::<Message, ()>::new();
    controller.spawn_worker(DataGenerator {})?;
    controller.spawn_worker(DataParser {})?;
    controller.spawn_worker(SignalHandler {})?;
    controller.spawn_worker(VeryBlocking {})?;
    info!("controller started");
    controller.block();
    info!("controller terminated");
    Ok(())
}
