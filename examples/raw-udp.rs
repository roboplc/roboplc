use roboplc::io::raw_udp::{UdpReceiver, UdpSender};
use roboplc::prelude::*;
use roboplc::time::interval;
use tracing::{error, info};

#[derive(DataPolicy, Clone)]
enum Message {
    Env(EnvData),
}

// A raw UDP structure, to be sent and received
//
// The recommended way of IPC for RoboPLC
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
    set_at: u64,
}

// A worker to collect data from incoming UDP packets
#[derive(WorkerOpts)]
#[worker_opts(name = "udp_in", blocking = true)]
struct UdpIn {}

impl Worker<Message, ()> for UdpIn {
    fn run(&mut self, context: &Context<Message, ()>) -> WResult {
        let rx = UdpReceiver::<EnvData>::bind("127.0.0.1:25000", 32)?;
        // [`UdpInput`] is an iterator of incoming UDP packets which are automatically parsed
        for data in rx {
            match data {
                Ok(data) => {
                    let latency = Monotonic::now() - Monotonic::from_nanos(data.set_at);
                    info!(worker = self.worker_name(), latency = ?latency);
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
    fn run(&mut self, context: &Context<Message, ()>) -> WResult {
        let mut tx = UdpSender::connect("localhost:25000")?;
        for _ in interval(Duration::from_secs(1)) {
            let data = EnvData {
                temp: 25.0,
                hum: 50.0,
                pressure: 1000.0,
                set_at: u64::try_from(Monotonic::now().as_nanos()).unwrap(),
            };
            if let Err(e) = tx.send(data) {
                error!(worker=self.worker_name(), error=%e, "udp send error");
            }
            if !context.is_online() {
                break;
            }
        }
        Ok(())
    }
}

// A worker to print data, received by the `UdpIn` worker
#[derive(WorkerOpts)]
#[worker_opts(name = "printEnv", blocking = true)]
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
    roboplc::setup_panic();
    roboplc::configure_logger(roboplc::LevelFilter::Info);
    // creates a controller instance
    let mut controller = Controller::<Message, ()>::new();
    // spawns workers
    controller.spawn_worker(UdpIn {})?;
    controller.spawn_worker(PrintEnv {})?;
    controller.spawn_worker(UdpOut {})?;
    // register SIGINT and SIGTERM signals with max shutdown timeout of 5 seconds
    controller.register_signals(Duration::from_secs(5))?;
    // blocks the main thread while the controller is online and the workers are running
    controller.block();
    Ok(())
}
