use roboplc::controller::prelude::*;
use roboplc::prelude::*;
use tracing::info;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

type Message = ();
type Variables = ();

#[derive(WorkerOpts)]
#[worker_opts(cpu = 0, priority = 50, scheduling = "fifo", blocking = true)]
struct Worker1 {}

impl Worker<Message, Variables> for Worker1 {
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        let channel = roboplc::rflow::take_data_channel()?;
        info!("Starting chat, connect to the local port 4001 with `rflow-chat`, `nc` or `telnet`");
        info!("Example: `echo \"Hello\" | nc -N localhost 4001`");
        for msg in channel {
            info!(%msg, "Received");
            roboplc::rflow::send(format!("RoboPLC received: {}", msg));
        }
        Ok(())
    }
}

#[derive(WorkerOpts)]
#[worker_opts(cpu = 0, priority = 50, scheduling = "fifo", blocking = true)]
struct RflowSrv {}

impl Worker<Message, Variables> for RflowSrv {
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        roboplc::serve_rflow().map_err(Into::into)
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
    controller.spawn_worker(RflowSrv {})?;
    controller.spawn_worker(Worker1 {})?;
    controller.register_signals(SHUTDOWN_TIMEOUT)?;
    controller.block();
    Ok(())
}
