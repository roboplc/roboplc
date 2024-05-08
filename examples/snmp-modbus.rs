/// An example of SNMP->Modbus TCP gateway for a 16-port relay board with SNMP-enabled control.
///
/// The program reads the current state of the relay board to coils 0-15 of the Modbus context
/// storage available as unit 1. If the coils are modified by a Modbus client, the program writes
/// the new state to the relay board. State changes are not written unless modified.
///
/// The discrete register 0 displays the relay board state. (0 - unavailable, 1 - ok)
use std::ops::Range;

use roboplc::controller::prelude::*;
use roboplc::io::modbus::{prelude::*, ModbusServerWritePermission};
use roboplc::locking::Mutex;
use roboplc::prelude::*;
use roboplc::time::interval;
use tracing::{error, info, warn};

const MODBUS_TIMEOUT: Duration = Duration::from_secs(1);
const MODBUS_LISTEN: &str = "0.0.0.0:5502";
const MODBUS_UNIT: u8 = 1;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

const SNMP_TIMEOUT: Duration = Duration::from_millis(400);

const RELAY_ADDR: &str = "10.210.110.26:161";
const RELAY_COMMUNITY: &[u8] = b"private";
/// Relay board lock, prevents concurrent access to Modbus coils 0-15
static RELAY_MODBUS_CONTEXT_LOCK: Mutex<()> = Mutex::new(());

type ModbusServerMapping = roboplc::io::modbus::ModbusServerMapping<16, 1, 0, 0>;
type ModbusServer = roboplc::io::modbus::ModbusServer<16, 1, 0, 0>;

/// A 16-port relay state
#[derive(Default, Clone)]
#[binrw]
struct Relays16 {
    ports: [u8; 16],
}

type Message = ();
type Variables = ();

#[derive(WorkerOpts)]
#[worker_opts(cpu = 2, priority = 50, scheduling = "fifo", blocking = true)]
struct Relay {
    port_mapping: ModbusServerMapping,
    state_mapping: ModbusServerMapping,
}

impl Worker<Message, Variables> for Relay {
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        let mut first_run = true;
        let mut sess = snmp::SyncSession::new(RELAY_ADDR, RELAY_COMMUNITY, Some(SNMP_TIMEOUT), 0)?;
        let relay_oid = &[1, 3, 6, 1, 4, 1, 42505, 6, 2, 3, 1, 3];
        let mut prev_relay_state = Relays16::default();
        let mut relay_down = false;
        for int_state in interval(Duration::from_millis(500)) {
            if !int_state {
                warn!("Relay worker loop timeout");
            }
            let _lock = RELAY_MODBUS_CONTEXT_LOCK.lock();
            let mut relays: Relays16 = self.port_mapping.read().unwrap_or_default();
            if first_run {
                // we do not have a previous state yet, so do not process any changes
                first_run = false;
            } else {
                // write changes to the relay board in case if Modbus context storage coils are
                // changed
                for (i, (prev, current)) in prev_relay_state
                    .ports
                    .iter()
                    .zip(relays.ports.iter())
                    .enumerate()
                {
                    if prev != current {
                        let port_oid = &[
                            1,
                            3,
                            6,
                            1,
                            4,
                            1,
                            42505,
                            6,
                            2,
                            3,
                            1,
                            3,
                            u32::try_from(i).unwrap(),
                        ];
                        let value = snmp::Value::Integer((*current).into());
                        match sess.set(&[(port_oid, value)]) {
                            Ok(res) => {
                                if res.error_status != snmp::snmp::ERRSTATUS_NOERROR {
                                    error!(status = res.error_status, "Relay SNMP set error");
                                }
                            }
                            Err(error) => {
                                error!(?error, "Relay SNMP set error");
                            }
                        }
                    }
                }
            }
            // read the current relay board state
            match sess.getbulk(&[relay_oid], 0, 16) {
                Ok(response) => {
                    for (name, val) in response.varbinds {
                        let snmp::Value::Integer(value) = val else {
                            continue;
                        };
                        let Ok(value) = u8::try_from(value) else {
                            continue;
                        };
                        let Some(port) = name.raw().last() else {
                            continue;
                        };
                        if usize::from(*port) >= relays.ports.len() {
                            continue;
                        }
                        relays.ports[usize::from(*port)] = value;
                    }
                    // save the current relay board state
                    prev_relay_state = relays.clone();
                    // write the current relay board state to the Modbus context storage
                    self.port_mapping.write(relays)?;
                    if relay_down {
                        self.state_mapping.write(1u8)?;
                        info!("Relay back online");
                        relay_down = false;
                    }
                }
                Err(error) => {
                    if !relay_down {
                        self.state_mapping.write(0u8)?;
                        error!(?error, "Relay down");
                        relay_down = true;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(WorkerOpts)]
#[worker_opts(cpu = 3, priority = 50, scheduling = "fifo", blocking = true)]
struct ModbusSrv {
    server: ModbusServer,
}

impl Worker<Message, Variables> for ModbusSrv {
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        self.server.serve()?;
        Ok(())
    }
}

fn relay_modbus_write_allow(
    kind: ModbusRegisterKind,
    range: Range<u16>,
) -> ModbusServerWritePermission {
    if kind == ModbusRegisterKind::Coil && range.end < 16 {
        ModbusServerWritePermission::AllowLock(RELAY_MODBUS_CONTEXT_LOCK.lock())
    } else {
        ModbusServerWritePermission::Allow
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    roboplc::setup_panic();
    roboplc::configure_logger(roboplc::LevelFilter::Info);
    if !roboplc::is_production() {
        roboplc::thread_rt::set_simulated();
    }
    roboplc::thread_rt::prealloc_heap(10_000_000)?;
    let mut server = ModbusServer::bind(
        roboplc::comm::Protocol::Tcp,
        MODBUS_UNIT,
        MODBUS_LISTEN,
        MODBUS_TIMEOUT,
        1,
    )?;
    server.set_allow_external_write_fn(relay_modbus_write_allow);
    let port_mapping = server.mapping("c@0".parse()?, 16);
    let mut state_mapping = server.mapping("d@0".parse()?, 1);
    state_mapping.write(1u8)?;
    let mut controller = Controller::<Message, Variables>::new();
    controller.spawn_worker(ModbusSrv { server })?;
    controller.spawn_worker(Relay {
        port_mapping,
        state_mapping,
    })?;
    controller.register_signals(SHUTDOWN_TIMEOUT)?;
    controller.block();
    Ok(())
}
