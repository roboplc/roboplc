// EVA ICS example
use roboplc::{
    io::eapi::{EAPIConfig, EAPI, OID},
    prelude::*,
    time::interval,
};
use serde::Deserialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tracing::info;

#[derive(Clone, Debug)]
#[binrw]
struct Env {
    temp: f64,
    hum: f64,
    pressure: f64,
}

#[derive(Default)]
struct Variables {
    fan: AtomicBool,
}

#[derive(DataPolicy, Clone)]
enum Message {}

#[derive(WorkerOpts)]
#[worker_opts(name = "worker1")]
struct Worker1 {
    eapi: EAPI<Message, Variables>,
}

#[allow(clippy::cast_lossless)]
impl Worker<Message, Variables> for Worker1 {
    fn run(&mut self, context: &Context<Message, Variables>) -> WResult {
        let mut temp = 25;
        let oid: Arc<OID> = "unit:tests/fan".parse::<OID>().unwrap().into();
        let dobj_name: Arc<String> = "Env".to_owned().into();
        for _ in interval(Duration::from_millis(200)).take_while(|_| context.is_online()) {
            if temp == 25 {
                temp = 10;
            } else {
                temp = 25;
            }
            info!(temp);
            self.eapi.dobj_push(
                dobj_name.clone(),
                Env {
                    temp: temp as f64,
                    hum: temp as f64 / 2.0,
                    pressure: temp as f64 / 3.0,
                },
            )?;
            self.eapi.state_push(
                oid.clone(),
                u8::from(context.variables().fan.load(Ordering::Acquire)),
            )?;
            //self.eapi.dobj_error(dobj_name.clone())?;
        }
        Ok(())
    }
}

// EAPI requires a separate connector worker to run with
#[derive(WorkerOpts)]
#[worker_opts(name = "eapi", blocking = true)]
struct EAPIConnector {
    eapi: EAPI<Message, Variables>,
}

impl Worker<Message, Variables> for EAPIConnector {
    fn run(&mut self, context: &Context<Message, Variables>) -> WResult {
        self.eapi.run(self.worker_name(), context);
        Ok(())
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // This macro call copies AUTHOR, VERSION and DESCRIPTION from program's Cargo.toml to the EAPI
    // I/O module.
    roboplc::init_eapi!();
    roboplc::setup_panic();
    roboplc::configure_logger(roboplc::LevelFilter::Info);
    let eapi_config: EAPIConfig<Message, Variables> = EAPIConfig::new("/opt/eva4/var/bus.ipc")
        .action_handler("unit:tests/fan".parse().unwrap(), |action, context| {
            let params = action.take_unit_params()?;
            let val = u8::deserialize(params.value)?;
            context.variables().fan.store(val != 0, Ordering::Release);
            Ok(None)
        });
    // this creates a connector instance with the name `fieldbus.HOSTNAME.plc`. To use a custom
    // name, use `EAPI::new` instead.
    let eapi = EAPI::new_program(eapi_config);
    let mut controller = Controller::<Message, Variables>::new();
    controller.register_signals(Duration::from_secs(5))?;
    controller.spawn_worker(Worker1 { eapi: eapi.clone() })?;
    controller.spawn_worker(EAPIConnector { eapi })?;
    controller.block();
    Ok(())
}
