use std::path::Path;
use std::sync::Arc;
use std::{collections::BTreeMap, io::Write as _};

use colored::Colorize;
#[cfg(not(target_os = "windows"))]
use tokio::io::AsyncReadExt as _;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::signal::unix::SignalKind;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum Input {
    Resize((usize, usize)),
    Terminate,
}

pub fn exec(
    url: &str,
    key: &str,
    file: &Path,
    force: bool,
    args: Vec<String>,
    env: BTreeMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(exec_remote(url, key, file, force, args, env))?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Output {
    Error(String),
    Terminated(i32),
}

#[derive(Serialize)]
struct ExecPayload<'a> {
    k: &'a str,
    force: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    env: BTreeMap<String, String>,
    term: ExecTerm,
}

#[derive(Serialize)]
struct ExecTerm {
    width: usize,
    height: usize,
    name: String,
}

#[allow(clippy::too_many_lines)]
async fn exec_remote(
    url: &str,
    key: &str,
    file: &Path,
    force: bool,
    args: Vec<String>,
    env: BTreeMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (ws_uri, url_short) = if let Some(u) = url.strip_prefix("http://") {
        (format!("ws://{}/roboplc/api/ws.execute", u), u)
    } else if let Some(u) = url.strip_prefix("https://") {
        (format!("wss://{}/roboplc/api/ws.execute", u), u)
    } else {
        return Err("Invalid URL".into());
    };
    println!("Executing on the remote host {}", url_short.green().bold());
    println!();
    let (mut socket, _) = tokio_tungstenite::connect_async(&ws_uri).await?;
    let (width, height) = term_size::dimensions().ok_or("Failed to get terminal size")?;
    let payload = ExecPayload {
        k: key,
        force,
        args,
        env,
        term: ExecTerm {
            width,
            height,
            name: std::env::var("TERM").unwrap_or("xterm-256color".to_string()),
        },
    };
    socket
        .send(Message::Text(serde_json::to_string(&payload)?))
        .await?;
    let Some(Ok(Message::Text(msg))) = socket.next().await else {
        return Err("Expected text message".into());
    };
    if msg != "upload" {
        if let Ok(Output::Error(e)) = serde_json::from_str::<Output>(&msg) {
            return Err(e.into());
        }
        return Err(format!("Unexpected message: {}", msg).into());
    }
    let f = tokio::fs::read(file).await?;
    socket.send(Message::Binary(f)).await?;
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    #[allow(unused_mut)]
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));
    // input on windows is currently not supported
    #[cfg(not(target_os = "windows"))]
    let input_fut = {
        let sender_c = sender.clone();
        tokio::spawn(async move {
            let stdin = std::os::fd::AsRawFd::as_raw_fd(&std::io::stdin().lock());
            let mut termios =
                termios::Termios::from_fd(stdin).expect("Failed to get termios for stdin");
            termios.c_lflag &= !(termios::ICANON | termios::ECHO);
            termios::tcsetattr(stdin, termios::TCSANOW, &termios)
                .expect("Failed to set termios for stdin");
            let mut f = unsafe { <tokio::fs::File as std::os::fd::FromRawFd>::from_raw_fd(stdin) };
            let buf = &mut [0u8; 4096];
            while let Ok(b) = f.read(buf).await {
                if b == 0 {
                    break;
                }
                if let Err(e) = sender_c
                    .lock()
                    .await
                    .send(Message::Binary(buf[..b].to_vec()))
                    .await
                {
                    eprintln!("Error sending input: {}", e);
                    break;
                }
            }
        })
    };
    // signal handler
    #[cfg(not(target_os = "windows"))]
    {
        macro_rules! handle_term_signal {
            ($sig: expr, $sender: expr) => {
                tokio::spawn(async move {
                    $sig.recv().await;
                    $sender
                        .lock()
                        .await
                        .send(Message::Text(
                            serde_json::to_string(&Input::Terminate).unwrap(),
                        ))
                        .await
                        .ok();
                });
            };
        }
        let mut sigint = tokio::signal::unix::signal(SignalKind::interrupt())?;
        let sender_c = sender.clone();
        handle_term_signal!(sigint, sender_c);
        let mut sigterm = tokio::signal::unix::signal(SignalKind::terminate())?;
        let sender_c = sender.clone();
        handle_term_signal!(sigterm, sender_c);
        let mut sighup = tokio::signal::unix::signal(SignalKind::hangup())?;
        let sender_c = sender.clone();
        handle_term_signal!(sighup, sender_c);
        let mut sigwinch = tokio::signal::unix::signal(SignalKind::window_change())?;
        let sender_c = sender.clone();
        tokio::spawn(async move {
            loop {
                sigwinch.recv().await;
                let Some(dimensions) = term_size::dimensions() else {
                    continue;
                };
                sender_c
                    .lock()
                    .await
                    .send(Message::Text(
                        serde_json::to_string(&Input::Resize(dimensions)).unwrap(),
                    ))
                    .await
                    .ok();
            }
        });
    }
    macro_rules! handle_out {
        ($out: expr) => {
            let Some(Ok(Message::Binary(b))) = receiver.next().await else {
                return Err("Expected binary message".into());
            };
            $out.write_all(&b)?;
            $out.flush()?;
        };
    }
    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(m) = msg {
            match m.as_str() {
                "o" => {
                    handle_out!(stdout);
                }
                "e" => {
                    handle_out!(stderr);
                }
                v => {
                    let output = serde_json::from_str::<Output>(v)?;
                    match output {
                        Output::Error(e) => {
                            eprintln!("Program error: {}", e);
                            break;
                        }
                        Output::Terminated(code) => {
                            if code == 0 {
                                std::process::exit(0);
                            } else {
                                eprintln!("Program terminated with code {}", code);
                                std::process::exit(code);
                            }
                        }
                    }
                }
            }
        }
    }
    // actually unreachable
    #[cfg(not(target_os = "windows"))]
    input_fut.abort();
    Ok(())
}
