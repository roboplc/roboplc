/// Launches a subprocess and reads its output line by line. Useful to connect RoboPLC with 3rd
/// party software which can not be embedded.
use roboplc::controller::prelude::*;
use roboplc::io::pipe::{self, Pipe};
use roboplc::{prelude::*, Error};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

type Message = ();
type Variables = ();

#[derive(WorkerOpts)]
#[worker_opts(cpu = 0, priority = 50, scheduling = "fifo", blocking = true)]
struct Worker1 {
    reader: pipe::Reader,
}

impl Worker<Message, Variables> for Worker1 {
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        loop {
            let line = self.reader.line()?;
            println!("Worker1: {}", line.trim_end());
        }
    }
}

#[derive(WorkerOpts)]
#[worker_opts(cpu = 0, priority = 50, scheduling = "fifo", blocking = true)]
struct PipeRunner {
    pipe: Pipe,
}

impl Worker<Message, Variables> for PipeRunner {
    /// The piped subprocess needs to be run by a worker. The subprocess inherits the scheduling
    /// policy and priority of the worker.
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        self.pipe.run();
        Err(Error::failed("pipe exited").into())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    roboplc::setup_panic();
    roboplc::configure_logger(roboplc::LevelFilter::Info);
    if !roboplc::is_production() {
        roboplc::thread_rt::set_simulated();
    }
    let _sys = roboplc::thread_rt::SystemConfig::new()
        .set("kernel/sched_rt_runtime_us", -1)
        .apply()
        .expect("Unable to set system config");
    roboplc::thread_rt::prealloc_heap(10_000_000)?;
    let mut controller = Controller::<Message, Variables>::new();
    let (pipe, reader) = Pipe::new("/path/to/subprogram");
    controller.spawn_worker(Worker1 { reader })?;
    controller.spawn_worker(PipeRunner { pipe })?;
    controller.register_signals(SHUTDOWN_TIMEOUT)?;
    controller.block();
    Ok(())
}
