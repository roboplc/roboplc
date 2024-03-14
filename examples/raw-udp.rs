use roboplc::io::raw_udp::{UdpInput, UdpOutput};
use roboplc::prelude::*;
use roboplc::time::interval;
use tracing::{error, info};

#[derive(DataPolicy, Clone)]
enum Message {
    Env(EnvData),
}

// A raw UDP structure, to be sent and received
//
// Raw UDP structures are used by various software, e.g. Matlab, LabView, etc. as well as by some
// fieldbus devices
#[derive(Debug, Clone)]
#[binrw]
#[brw(little)]
struct EnvData {
    temp: f64,
    hum: f64,
    pressure: f64,
}

// A worker to collect data from incoming UDP packets
#[derive(WorkerOpts)]
#[worker_opts(name = "udp_in")]
struct UdpIn {}

impl Worker<Message, ()> for UdpIn {
    fn run(&mut self, context: &Context<Message, ()>) -> WResult {
        let server = UdpInput::<EnvData>::bind("127.0.0.1:25000", 24)?;
        // [`UdpInput`] is an iterator of incoming UDP packets which are automatically parsed
        for data in server {
            match data {
                Ok(data) => {
                    context.hub().send(Message::Env(data));
                }
                Err(e) => {
                    error!(worker=self.worker_name(), error=%e, "udp in error");
                }
            }
        }
        Ok(())
    }
}

// A worker to send data to a remote UDP server
// (in this example data is just sent to UDP input worker)
#[derive(WorkerOpts)]
#[worker_opts(name = "udp_out")]
struct UdpOut {}

impl Worker<Message, ()> for UdpOut {
    fn run(&mut self, _context: &Context<Message, ()>) -> WResult {
        let mut client = UdpOutput::connect("localhost:25000")?;
        for _ in interval(Duration::from_secs(1)) {
            let data = EnvData {
                temp: 25.0,
                hum: 50.0,
                pressure: 1000.0,
            };
            if let Err(e) = client.send(data) {
                error!(worker=self.worker_name(), error=%e, "udp send error");
            }
        }
        Ok(())
    }
}

// A worker to print data, received by the `UdpIn` worker
#[derive(WorkerOpts)]
#[worker_opts(name = "printEnv")]
struct PrintEnv {}

impl Worker<Message, ()> for PrintEnv {
    fn run(&mut self, context: &Context<Message, ()>) -> WResult {
        let hc = context
            .hub()
            .register(self.worker_name(), event_matches!(Message::Env(_)))?;
        for msg in hc {
            let Message::Env(data) = msg;
            info!(worker = self.worker_name(), data=?data);
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // sets the simulated mode for the real-time module, do not set any thread real-time parameters
    roboplc::thread_rt::set_simulated();
    // initializes a debug logger
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    // creates a controller instance
    let mut controller = Controller::<Message, ()>::new();
    // spawns workers
    controller.spawn_worker(UdpIn {})?;
    controller.spawn_worker(PrintEnv {})?;
    controller.spawn_worker(UdpOut {})?;
    // block the main thread until the controller is in the online state
    controller.block_while_online();
    Ok(())
}
