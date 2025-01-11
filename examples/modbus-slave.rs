use roboplc::comm::Protocol;
use roboplc::io::modbus::prelude::*;
use roboplc::locking::Mutex;
use roboplc::{prelude::*, time::interval};
use tracing::info;

/// Modbus slave storage context size for each register type
const HOLDINGS: usize = 2;
const DISCRETES: usize = 0;
const INPUTS: usize = 13;
const COILS: usize = 2;

/// A server mapping alias
type ServerMapping = ModbusServerMapping<COILS, DISCRETES, INPUTS, HOLDINGS>;

/// First data type. For Modbus slave context data types must be split into booleans and others
#[derive(Clone, Debug, Default)]
#[binrw]
struct Data {
    counter: u16, // available in the input register 0 (i0)
}

/// Booleans data type
#[derive(Clone, Debug, Default)]
#[binrw]
struct Relays {
    relay1: u8, // available in the coil 0 (c0)
    relay2: u8, // available in the coil 1 (c1)
}

#[derive(Default)]
#[binrw]
struct Input {
    value: u32,
}

// This example does not use controller's data hub
type Message = ();
type Variables = Mutex<VariableData>;

// Controller's shared variables
#[derive(Default)]
struct VariableData {
    data: Data,
    relays: Relays,
    input: Input,
}

#[derive(WorkerOpts)]
#[allow(clippy::struct_field_names)]
struct Worker1 {
    // Modbus server context and controller variables/data hub are not synchronized automatically,
    // workers must write to the Modbus server context and read from the controller variables/data
    // hub
    env_mapping: ServerMapping,
    relay_mapping: ServerMapping,
    input_mapping: ServerMapping,
}

#[allow(clippy::cast_lossless)]
impl Worker<Message, Variables> for Worker1 {
    fn run(&mut self, context: &Context<Message, Variables>) -> WResult {
        for _ in interval(Duration::from_secs(2)).take_while(|_| context.is_online()) {
            let mut vars = context.variables().lock();
            vars.data.counter += 1;
            vars.relays.relay1 = u8::from(vars.data.counter % 2 == 0);
            vars.relays.relay2 = u8::from(vars.data.counter % 2 != 0);
            self.env_mapping.write(&vars.data)?;
            self.relay_mapping.write(&vars.relays)?;
            vars.input = self.input_mapping.read()?;
            info!(%vars.data.counter, "i0(1)");
            info!(%vars.relays.relay1, "c0(1)");
            info!(%vars.relays.relay2, "c1(1)");
            info!(%vars.input.value, "h0(2)");
        }
        Ok(())
    }
}

/// Modbus server requires a worker to run with
#[derive(WorkerOpts)]
#[worker_opts(blocking = true)]
struct ModbusSrv {
    server: ModbusServer<COILS, DISCRETES, INPUTS, HOLDINGS>,
}

impl Worker<Message, Variables> for ModbusSrv {
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        self.server.serve()?;
        Ok(())
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    roboplc::setup_panic();
    roboplc::configure_logger(roboplc::LevelFilter::Info);
    // for TCP
    let addr = "0.0.0.0:5552";
    // for RTU
    //let addr = "/dev/ttyS0:9600:8:N:1";
    // Modbus Unit ID
    let unit = 1;
    let timeout = Duration::from_secs(5);
    let server = ModbusServer::bind(Protocol::Tcp, unit, addr, timeout, 1)?;
    // Modbus server register mapping, can be specified with '@' or without
    let env_mapping = server.mapping("i@0".parse()?, 13);
    let relay_mapping = server.mapping("c@0".parse()?, 2);
    let input_mapping = server.mapping("h@0".parse()?, 2);
    let mut controller = Controller::<Message, Variables>::new();
    controller.register_signals(Duration::from_secs(5))?;
    controller.spawn_worker(Worker1 {
        env_mapping,
        relay_mapping,
        input_mapping,
    })?;
    controller.spawn_worker(ModbusSrv { server })?;
    info!(addr, unit, "started");
    controller.block();
    info!("exiting");
    Ok(())
}
