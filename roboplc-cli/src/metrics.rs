use std::{
    collections::BTreeMap,
    io::{BufRead as _, BufReader},
};

use prettytable::{format, row, Table};
use ureq::Agent;

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub fn display(url: &str, port: u16, agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    let mut url = url::Url::parse(url)?;
    url.set_port(Some(port)).map_err(|()| "invalid port")?;
    let r = agent.get(url.as_str()).call()?;
    if r.status() != 200 {
        return Err(format!("Error: {}", r.status()).into());
    }
    let r = BufReader::new(r.into_reader());
    let mut values = BTreeMap::new();
    for line in r.lines() {
        let line = line?;
        let mut l = line.split('#').next().unwrap_or("");
        l = l.trim();
        if l.is_empty() {
            continue;
        }
        let mut sp = l.splitn(2, ' ');
        let name = sp.next().unwrap();
        let value = sp.next().unwrap_or("");
        values.insert(name.to_string(), value.to_string());
    }
    let mut table = Table::new();
    let format = format::FormatBuilder::new()
        .column_separator(' ')
        .padding(1, 1)
        .build();
    table.set_format(format);
    for (key, value) in values {
        table.add_row(row![key, value]);
    }
    table.printstd();
    Ok(())
}
