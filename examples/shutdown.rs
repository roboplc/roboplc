// The example provides a graceful shutdown of the controller using built-in methods.

use roboplc::{prelude::*, time::interval};
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
fn main() -> Result<(), Box<dyn std::error::Error>> {
    roboplc::setup_panic();
    roboplc::configure_logger(roboplc::LevelFilter::Info);
    let mut controller = Controller::<Message, ()>::new();
    controller.spawn_worker(DataGenerator {})?;
    controller.spawn_worker(DataParser {})?;
    controller.spawn_worker(VeryBlocking {})?;
    controller.register_signals_with_handlers(
        move |context| {
            context.hub().send(Message::Terminate);
        },
        |_| {
            info!("Allowing reload");
            Ok(())
        },
        SHUTDOWN_TIMEOUT,
    )?;
    info!("controller started");
    controller.block();
    info!("controller terminated");
    Ok(())
}
