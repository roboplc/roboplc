// EVA ICS example
use roboplc::{
    io::eapi::{EAPIConfig, EAPI, OID},
    prelude::*,
    time::interval,
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;

#[derive(Clone, Debug)]
#[repr(C)]
#[binrw]
struct Env {
    temp: f64,
    hum: f64,
    pressure: f64,
}

#[derive(Default)]
struct Variables {
    fan: bool,
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
        for _ in interval(Duration::from_millis(200)) {
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
            self.eapi
                .state_push(oid.clone(), u8::from(context.variables().read().fan))?;
            //self.eapi.dobj_error(dobj_name.clone())?;
            if !context.is_online() {
                break;
            }
        }
        Ok(())
    }
}

// EAPI requires a separate connector worker
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
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    let eapi_config: EAPIConfig<Message, Variables> = EAPIConfig::new("/opt/eva4/var/bus.ipc")
        .action_handler("unit:tests/fan".parse().unwrap(), |action, context| {
            let params = action.take_unit_params()?;
            let val = u8::deserialize(params.value)?;
            context.variables().write().fan = val != 0;
            Ok(())
        });
    let eapi = EAPI::new("fieldbus.host1.plc.test", eapi_config);
    let mut controller = Controller::<Message, Variables>::new();
    controller.register_signals(Duration::from_secs(5))?;
    controller.spawn_worker(Worker1 { eapi: eapi.clone() })?;
    controller.spawn_worker(EAPIConnector { eapi })?;
    controller.block();
    Ok(())
}
