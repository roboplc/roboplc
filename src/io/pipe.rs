//! Data processing with subprocesses
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io,
    process::Stdio,
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    process::Command,
};
use tracing::error;

use crate::{
    policy_channel_async::{self as pchannel_async, Receiver},
    DataDeliveryPolicy, Result,
};

/// Pipe reader
pub struct Reader {
    rx: Receiver<String>,
}

impl Reader {
    /// Reads a line from the pipe. Blocks until a line is available.
    pub fn line(&self) -> Result<String> {
        self.rx.recv_blocking().map_err(Into::into)
    }
}

/// Data pipe with a subprocess
pub struct Pipe {
    program: OsString,
    args: Vec<OsString>,
    environment: BTreeMap<String, String>,
    input_data: Option<Vec<u8>>,
    tx: pchannel_async::Sender<String>,
    restart_delay: Duration,
}

impl Pipe {
    /// Creates a new pipe with a subprocess
    pub fn new<P: AsRef<OsStr>>(program: P) -> (Self, Reader) {
        let (tx, rx) = pchannel_async::bounded(10);
        (
            Self {
                program: program.as_ref().to_owned(),
                args: Vec::new(),
                environment: BTreeMap::new(),
                input_data: None,
                tx,
                restart_delay: Duration::from_secs(1),
            },
            Reader { rx },
        )
    }
    /// Adds a command line argument
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }
    /// Adds multiple command line arguments
    pub fn args(mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        self.args
            .extend(args.into_iter().map(|x| x.as_ref().to_owned()));
        self
    }
    /// Adds an environment variable
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }
    /// Adds multiple environment variables
    pub fn envs(
        mut self,
        envs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.environment
            .extend(envs.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }
    /// STDIN data for the subprocess
    pub fn input_data(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.input_data = Some(data.into());
        self
    }
    /// Delay before restarting the subprocess after it terminates
    pub fn restart_delay(mut self, delay: Duration) -> Self {
        self.restart_delay = delay;
        self
    }
    /// Launches a subprocess pipe. The subprocess is restarted automatically if it terminates. The
    /// subprocess inherits sheduling policy and priority of the parent thread.
    ///
    /// # Panics
    ///
    /// Will panic if the method is unable to create tokio runtime
    pub fn run(&self) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(self.run_async());
    }
    async fn run_async(&self) {
        loop {
            match command_pipe(
                &self.program,
                &self.args,
                &Options {
                    environment: self.environment.clone(),
                    input_data: self.input_data.clone(),
                },
            ) {
                Ok(rx) => {
                    while let Ok(v) = rx.recv().await {
                        match v {
                            CommandPipeOutput::Stdout(line) => {
                                if self.tx.send(line).await.is_err() {
                                    return;
                                }
                            }
                            CommandPipeOutput::Stderr(line) => {
                                error!(program=%self.program.to_string_lossy(), "{}",
                                    line.trim_end());
                            }
                            CommandPipeOutput::Terminated(code) => {
                                if code != 0 {
                                    error!(program=%self.program.to_string_lossy(), "Command terminated with code {}", code);
                                }
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    error!(program=%self.program.to_string_lossy(), %error, "Failed to start command pipe");
                }
            }
            tokio::time::sleep(self.restart_delay).await;
        }
    }
}

#[derive(Default, Clone)]
struct Options {
    environment: BTreeMap<String, String>,
    input_data: Option<Vec<u8>>,
}

#[derive(Debug)]
enum CommandPipeOutput {
    Stdout(String),
    Stderr(String),
    Terminated(i32),
}

impl DataDeliveryPolicy for CommandPipeOutput {}

fn command_pipe<P, I, S>(
    program: P,
    args: I,
    opts: &Options,
) -> io::Result<Receiver<CommandPipeOutput>>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let (output_tx, output_rx) = pchannel_async::bounded(10);

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .envs(&opts.environment)
        .spawn()?;
    let stdin = if opts.input_data.is_some() {
        match child.stdin.take() {
            Some(v) => Some(v),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Unable to create stdin writer",
                ))
            }
        }
    } else {
        None
    };
    let stdin_writer = stdin.map(BufWriter::new);
    let stderr = child.stderr.take().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "Failed to capture stderr of child process",
        )
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "Failed to capture stdout of child process",
        )
    })?;
    let fut_stdin = stdin_writer.map(|mut writer| {
        let input_data = opts.input_data.as_ref().unwrap().clone();
        tokio::spawn(async move {
            if let Err(error) = writer.write_all(&input_data).await {
                error!(%error, "Unable to write to stdin");
            } else if let Err(error) = writer.flush().await {
                error!(%error, "Unable to flush stdin");
            }
        })
    });

    tokio::spawn(async move {
        let output_tx_stderr = output_tx.clone();

        let stderr_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while reader.read_line(&mut line).await.is_ok() {
                if line.is_empty()
                    || (output_tx_stderr
                        .send(CommandPipeOutput::Stderr(line.clone()))
                        .await)
                        .is_err()
                {
                    break;
                }
                line.clear();
            }
        });

        let output_tx_stdout = output_tx.clone();

        let stdout_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while reader.read_line(&mut line).await.is_ok() {
                if line.is_empty()
                    || (output_tx_stdout
                        .send(CommandPipeOutput::Stdout(line.clone()))
                        .await)
                        .is_err()
                {
                    break;
                }
                line.clear();
            }
        });

        let mut exit_code = 0;
        if let Ok(x) = child.wait().await {
            if let Some(code) = x.code() {
                exit_code = code;
            }
        }
        if let Some(v) = fut_stdin {
            v.abort();
        }
        tokio::select!(
            _ = stderr_handle => {},
            _ = stdout_handle => {},
        );
        let _r = output_tx
            .send(CommandPipeOutput::Terminated(exit_code))
            .await;
    });

    Ok(output_rx)
}
