//! The `command` driver (`docs/config.md`, "Driver I/O Contract (protocol 1)").
//!
//! Run operations only on a worker thread: never from an input callback or for a
//! pointer motion (IN-2).

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::config::{CommandOptions, Driver, Operation};
use crate::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use crate::platform::DisplayInfo;
use crate::pointer::Rect;

/// The stdout and stderr limit of a helper.
pub const OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationKind {
    PortalMatch,
    InputTarget,
    Handoff,
    Verify,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceRef {
    pub id: DeviceId,
    pub alias: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortalInfo {
    pub id: String,
    pub bounds: Option<Rect>,
    pub coordinate_space: &'static str,
    pub display_token: Option<DisplayToken>,
}

/// The request object on stdin. `operation` and `protocol` are added by [`run`].
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub computer_id: ComputerId,
    pub device: DeviceRef,
    pub source_computer_id: ComputerId,
    pub target_computer_id: Option<ComputerId>,
    pub session_id: Option<SessionId>,
    pub portal: Option<PortalInfo>,
    pub displays: Vec<DisplayInfo>,
}

#[derive(Serialize)]
struct Envelope<'a> {
    protocol: u32,
    operation: OperationKind,
    #[serde(flatten)]
    request: &'a Request,
}

/// The typed result of an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `portal-match` and `verify`.
    Matched(bool),
    /// `input-target`: a token of one of the request displays.
    Display(DisplayToken),
    /// `handoff`.
    Done,
}

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error("driver {0} is not supported by this build")]
    Unsupported(String),
    #[error("cannot start {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("I/O with the helper: {0}")]
    Io(#[from] std::io::Error),
    #[error("timeout after {0:?}")]
    Timeout(Duration),
    #[error("the helper exited with {status}: {stderr}")]
    Exit { status: String, stderr: String },
    #[error("{0} is larger than 64 KiB")]
    OutputTooLarge(&'static str),
    #[error("the result is not valid: {0}")]
    BadResult(String),
}

/// Runs one operation and checks the result shape.
pub fn run(
    operation: &Operation,
    kind: OperationKind,
    request: &Request,
    default_cwd: &Path,
) -> Result<Outcome, DriverError> {
    let options = match &operation.driver {
        Driver::Command(options) => options,
        Driver::Extension { name, .. } => return Err(DriverError::Unsupported(name.clone())),
    };
    let input = serde_json::to_vec(&Envelope {
        protocol: 1,
        operation: kind,
        request,
    })
    .map_err(|e| DriverError::BadResult(e.to_string()))?;
    let stdout = run_command(options, &input, operation.timeout, default_cwd)?;
    parse_result(kind, &stdout, &request.displays)
}

fn parse_result(
    kind: OperationKind,
    stdout: &[u8],
    displays: &[DisplayInfo],
) -> Result<Outcome, DriverError> {
    let value: Value = serde_json::from_slice(stdout)
        .map_err(|e| DriverError::BadResult(format!("stdout is not JSON: {e}")))?;
    let object = value
        .as_object()
        .filter(|o| o.len() == 1)
        .ok_or_else(|| DriverError::BadResult(format!("expected one field, got {value}")))?;
    let bad = || DriverError::BadResult(format!("wrong result for {kind:?}: {value}"));
    match kind {
        OperationKind::PortalMatch | OperationKind::Verify => object
            .get("matched")
            .and_then(Value::as_bool)
            .map(Outcome::Matched)
            .ok_or_else(bad),
        OperationKind::Handoff => match object.get("ok") {
            Some(Value::Bool(true)) => Ok(Outcome::Done),
            _ => Err(bad()),
        },
        OperationKind::InputTarget => {
            let token = object
                .get("display_token")
                .and_then(Value::as_str)
                .ok_or_else(bad)?;
            if displays.iter().any(|d| d.token.0 == token) {
                Ok(Outcome::Display(DisplayToken(token.to_owned())))
            } else {
                Err(DriverError::BadResult(format!(
                    "stale or unknown display token {token}"
                )))
            }
        }
    }
}

fn run_command(
    options: &CommandOptions,
    input: &[u8],
    timeout: Duration,
    default_cwd: &Path,
) -> Result<Vec<u8>, DriverError> {
    let deadline = Instant::now() + timeout;
    let (program, args) = options
        .argv
        .split_first()
        .ok_or_else(|| DriverError::BadResult("argv is empty".into()))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(options.cwd.as_deref().unwrap_or(default_cwd))
        .envs(&options.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A new process group, so that a timeout stops the whole process tree.
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|source| DriverError::Spawn {
        program: program.clone(),
        source,
    })?;
    let group = ProcessGroup(child.id());

    let stdout = reader(child.stdout.take());
    let stderr = reader(child.stderr.take());
    if let Some(mut stdin) = child.stdin.take() {
        // A helper can exit without reading stdin; that is not an error here.
        let _ = stdin.write_all(input);
    }

    let status = match wait(&mut child, deadline) {
        Some(status) => status,
        None => {
            group.kill();
            let _ = child.wait();
            return Err(DriverError::Timeout(timeout));
        }
    };
    let collect = |handle: thread::JoinHandle<Result<Vec<u8>, ()>>, name| {
        while !handle.is_finished() {
            if Instant::now() >= deadline {
                // A background process of the helper still holds the pipe.
                group.kill();
            }
            thread::sleep(Duration::from_millis(5));
        }
        handle
            .join()
            .ok()
            .and_then(Result::ok)
            .ok_or(DriverError::OutputTooLarge(name))
    };
    let out = collect(stdout, "stdout")?;
    let err = collect(stderr, "stderr")?;
    if !status.success() {
        return Err(DriverError::Exit {
            status: status.to_string(),
            stderr: String::from_utf8_lossy(&err).trim().to_owned(),
        });
    }
    Ok(out)
}

fn reader<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<Result<Vec<u8>, ()>> {
    thread::spawn(move || {
        let mut data = Vec::new();
        if let Some(pipe) = pipe {
            let mut limited = pipe.take(OUTPUT_LIMIT as u64 + 1);
            limited.read_to_end(&mut data).map_err(|_| ())?;
            // Drain the rest so that the helper does not block on a full pipe.
            let mut rest = limited.into_inner();
            let _ = std::io::copy(&mut rest, &mut std::io::sink());
        }
        if data.len() > OUTPUT_LIMIT {
            Err(())
        } else {
            Ok(data)
        }
    })
}

fn wait(child: &mut Child, deadline: Instant) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => return None,
        }
    }
}

struct ProcessGroup(u32);

impl ProcessGroup {
    fn kill(&self) {
        #[cfg(unix)]
        if let Ok(pgid) = libc::pid_t::try_from(self.0) {
            // SAFETY: killpg only sends a signal. The group is the one that
            // `process_group(0)` made for this helper.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }
}
