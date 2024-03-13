use roboplc::{
    comm::{tcp, Client},
    time::interval,
};
use roboplc::{io::modbus::prelude::*, prelude::*};
use tracing::{error, info, warn};

const MODBUS_TIMEOUT: Duration = Duration::from_secs(1);

// Do not make any decision if the sensor is older than this
const ENV_DATA_TTL: Duration = Duration::from_millis(1);

// A shared traditional PLC context, does not used for logic here but provided as an example
#[derive(Default)]
struct Variables {
    temperature: f32,
}

// A structure, fetched from Modbus registers
#[derive(Clone, Debug)]
#[binrw]
struct EnvironmentSensors {
    temperature: f32,
}

// A structue, to be written to Modbus registers
#[binrw]
struct Relay1 {
    fan1: u8,
    fan2: u8,
}

// Controller message type
#[derive(Clone, DataPolicy, Debug)]
enum Message {
    #[data_delivery(single)]
    #[data_expires(TtlCell::is_expired)]
    EnvSensorData(TtlCell<EnvironmentSensors>),
}

// First worker, to pull data from Modbus
#[derive(WorkerOpts)]
#[worker_opts(name = "puller", cpu = 1, scheduling = "fifo", priority = 80)]
struct ModbusPuller1 {
    sensor_mapping: ModbusMapping,
}

impl ModbusPuller1 {
    fn create(modbus_client: &Client) -> Result<Self, Box<dyn std::error::Error>> {
        // creates a Modbus register mapping for a structure
        // Modbus registers do not need to be parsed manually
        // RoboPLC fieldbus mappings use [binrw](https://crates.io/crates/binrw) crate for
        // automatic parsing
        let sensor_mapping = ModbusMapping::create(modbus_client, 2, "h0", 2)?;
        Ok(Self { sensor_mapping })
    }
}

// A worker implementation, contains a single function to run which has got access to the
// controller context
impl Worker<Message, Variables> for ModbusPuller1 {
    fn run(&mut self, context: &Context<Message, Variables>) {
        // this worker does not need to be subscribed to any events
        let hub = context.hub();
        for _ in interval(Duration::from_millis(500)) {
            match self.sensor_mapping.read::<EnvironmentSensors>() {
                Ok(v) => {
                    context.variables().lock().temperature = v.temperature;
                    hub.send(Message::EnvSensorData(TtlCell::new_with_value(
                        ENV_DATA_TTL,
                        v,
                    )));
                }
                Err(e) => {
                    error!(worker=self.worker_name(), err=%e, "Modbus pull error");
                }
            }
        }
    }
}

// Second worker, to control relays
#[derive(WorkerOpts)]
#[worker_opts(name = "relays", cpu = 2, scheduling = "fifo", priority = 80)]
struct ModbusRelays1 {
    fan_mapping: ModbusMapping,
}

impl ModbusRelays1 {
    fn create(modbus_client: &Client) -> Result<Self, Box<dyn std::error::Error>> {
        let fan_mapping = ModbusMapping::create(modbus_client, 3, "c2", 2)?;
        Ok(Self { fan_mapping })
    }
}

impl Worker<Message, Variables> for ModbusRelays1 {
    fn run(&mut self, context: &Context<Message, Variables>) {
        // this worker needs to be subscribed to EnvSensorData kind of events
        let hc = context
            .hub()
            .register(
                self.worker_name(),
                event_matches!(Message::EnvSensorData(_)),
            )
            .unwrap();
        for msg in hc {
            match msg {
                Message::EnvSensorData(mut cell) => {
                    // if data is not expired
                    if let Some(s) = cell.take() {
                        info!(worker=self.worker_name(), value=%s.temperature,
                            elapsed=?cell.set_at().elapsed());
                        // apply some logic: if temperature is above 30, turn on both fans, if
                        // below 25, turn off both fans, otherwise do nothing (keep hysterisis)
                        let relay = if s.temperature > 30.0 {
                            Some(Relay1 { fan1: 1, fan2: 1 })
                        } else if s.temperature < 25.0 {
                            Some(Relay1 { fan1: 0, fan2: 0 })
                        } else {
                            None
                        };
                        if let Some(r) = relay {
                            if let Err(e) = self.fan_mapping.write(&r) {
                                error!(worker=self.worker_name(), err=%e, "Modbus send error");
                            }
                        }
                    } else {
                        // Should never happen as policy channels do not deliver expired data if
                        // the policy specifies an expiration condition
                        warn!(worker=self.worker_name(), elapsed=?cell.set_at().elapsed(), "Expired data");
                    }
                }
            }
        }
    }
}

// Main function, to start the controller and workers
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // sets the simulated mode for the real-time module, do not set any thread real-time parameters
    roboplc::thread_rt::set_simulated();
    // initializes a debug logger
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    // creates a controller instance
    let mut controller: Controller<Message, Variables> = Controller::new();
    // creates a reliable auto-reconnecting shared TCP port connection
    let modbus_tcp_client = tcp::connect("10.90.34.111:5505", MODBUS_TIMEOUT)?;
    // creates the first worker and spawns it
    let worker = ModbusPuller1::create(&modbus_tcp_client)?;
    controller.spawn_worker(worker)?;
    // creates the second worker and spawns it
    let worker = ModbusRelays1::create(&modbus_tcp_client)?;
    controller.spawn_worker(worker)?;
    // block the main thread until the controller is in the online state
    controller.block_while_online();
    Ok(())
}
