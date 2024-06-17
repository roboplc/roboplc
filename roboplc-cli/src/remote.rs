use ureq::Agent;

use crate::{
    common::{report_ok, Mode, State},
    ureq_err::{self, PrintErr},
    API_PREFIX,
};

pub fn stat(url: &str, key: &str, agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    let resp = agent
        .post(&format!("{}{}/query.stats.program", url, API_PREFIX))
        .set("x-auth-key", key)
        .call()
        .process_error()?;
    let stats: State = resp.into_json()?;
    stats.print_std();
    Ok(())
}

pub fn set_mode(
    url: &str,
    key: &str,
    agent: &Agent,
    mode: Mode,
    report: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    agent
        .post(&format!("{}{}/set.program.mode", url, API_PREFIX))
        .set("x-auth-key", key)
        .send_json(ureq::json!({
             "mode": mode,
        }))
        .process_error()?;
    if report {
        report_ok()?;
    }
    Ok(())
}

pub fn purge(url: &str, key: &str, agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    ureq_err::PrintErr::process_error(
        agent
            .post(&format!("{}{}/purge.program.data", url, API_PREFIX))
            .set("x-auth-key", key)
            .call(),
    )?;
    report_ok()
}
