use std::io::Write as _;
use std::path::Path;

use colored::Colorize;
#[cfg(not(target_os = "windows"))]
use tokio::io::AsyncReadExt as _;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

pub fn exec(
    url: &str,
    key: &str,
    file: &Path,
    force: bool,
    args: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(exec_remote(url, key, file, force, args))?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Output {
    Error(String),
    Terminated(i32),
}

#[allow(clippy::too_many_lines)]
async fn exec_remote(
    url: &str,
    key: &str,
    file: &Path,
    force: bool,
    args: Vec<String>,
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
    let payload = json!({
        "k": key,
        "force": force,
        "args": args,
        "term": {
            "width": width,
            "height": height,
            "name": std::env::var("TERM").unwrap_or("xterm-256color".to_string()),
        },
    });
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
    let (mut sender, mut receiver) = socket.split();
    #[cfg(target_os = "windows")]
    let _ = sender;
    // input on windows is currently not supported
    #[cfg(not(target_os = "windows"))]
    let input_fut = tokio::spawn(async move {
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
            if let Err(e) = sender.send(Message::Binary(buf[..b].to_vec())).await {
                eprintln!("Error sending input: {}", e);
                break;
            }
        }
    });
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
