use bma_ts::Timestamp;
use prettytable::{cell, row, Table};
use serde::Deserialize;
use ureq::Agent;

use crate::{
    common::{report_ok, Mode, State},
    ureq_err::{self, PrintErr},
    API_PREFIX,
};

pub fn stat(
    url: &str,
    key: &str,
    agent: Agent,
    show_versions: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let resp = agent
        .post(&format!("{}{}/query.stats.program", url, API_PREFIX))
        .set("x-auth-key", key)
        .call()
        .process_error()?;
    let stats: State = resp.into_json()?;
    stats.print_std();
    if show_versions {
        println!();
        let resp = agent
            .post(&format!("{}{}/query.program.meta", url, API_PREFIX))
            .set("x-auth-key", key)
            .call()
            .process_error()?;
        let meta: PlcMetadata = resp.into_json()?;
        let mut table = Table::new();
        table.add_row(row!["Program", "Exists", "Created"]);
        table.add_row(row![
            "current",
            meta.program_current.exists_as_cell(),
            meta.program_current.created_iso()?
        ]);
        for (i, program) in meta.program_previous.iter().enumerate() {
            table.add_row(row![
                format!("prev.{}", i),
                program.exists_as_cell(),
                program.created_iso()?
            ]);
        }
        table.printstd();
    }
    Ok(())
}

#[derive(Deserialize)]
struct PlcMetadata {
    program_current: ProgramFileMetdata,
    #[serde(default)]
    program_previous: Vec<ProgramFileMetdata>,
}

#[derive(Deserialize)]
struct ProgramFileMetdata {
    exists: bool,
    created: Timestamp,
}

impl ProgramFileMetdata {
    fn exists_as_cell(&self) -> prettytable::Cell {
        if self.exists {
            cell!("YES")
        } else {
            cell!(Fr->"NO")
        }
    }
    fn created_iso(&self) -> Result<String, Box<dyn std::error::Error>> {
        Ok(self
            .created
            .try_into_datetime_local()?
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    }
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
