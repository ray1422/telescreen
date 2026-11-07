//! The `exec` and `command` drivers and the program search path (`docs/config.md`).
//!
//! Run operations only on a worker thread: never from an input callback or for a
//! pointer motion (IN-2).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::config::{Driver, Operation, Program};
use crate::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use crate::platform::DisplayInfo;
use crate::pointer::Rect;

/// The stdout and stderr limit of a helper.
pub const OUTPUT_LIMIT: usize = 64 * 1024;

/// The directories searched for a program name, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchPath(pub Vec<PathBuf>);

impl SearchPath {
    /// The `PATH` of this process. Empty if `PATH` is not set.
    pub fn from_env() -> SearchPath {
        SearchPath(
            std::env::var_os("PATH")
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default(),
        )
    }

    /// The first executable file `program` in the directories. A program with a
    /// `/` is returned unchanged, with no search.
    pub fn find(&self, program: &str) -> Option<PathBuf> {
        if program.contains('/') {
            return Some(PathBuf::from(program));
        }
        self.0
            .iter()
            .map(|directory| directory.join(program))
            .find(|path| {
                let Ok(metadata) = path.metadata() else {
                    return false;
                };
                if !metadata.is_file() {
                    return false;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o111 != 0
                }
                #[cfg(not(unix))]
                {
                    true
                }
            })
    }

    /// The `PATH` of the login shell: `shell -l -c` with a script that prints it
    /// between two markers, because a login shell can print other text first.
    pub fn from_shell(shell: &Path, timeout: Duration) -> Result<SearchPath, DriverError> {
        const BEGIN: &str = "__TELESCREEN_PATH_BEGIN__";
        const END: &str = "__TELESCREEN_PATH_END__";
        let script = format!("printf '%s%s%s' '{BEGIN}' \"$PATH\" '{END}'");
        let mut command = Command::new(shell);
        command.args(["-l", "-c", &script]);
        let output = run_process(&mut command, &shell.to_string_lossy(), None, true, timeout)?;
        check_exit(&output)?;
        let stdout = std::str::from_utf8(&output.stdout)
            .map_err(|e| DriverError::BadResult(format!("shell stdout is not UTF-8: {e}")))?;
        let path = stdout
            .split_once(BEGIN)
            .and_then(|(_, rest)| rest.split_once(END))
            .map(|(path, _)| path)
            .ok_or_else(|| DriverError::BadResult("shell PATH markers are missing".into()))?;
        Ok(SearchPath(
            std::env::split_paths(path)
                .filter(|directory| !directory.as_os_str().is_empty())
                .collect(),
        ))
    }
}

/// The working directory and program search path of driver jobs.
#[derive(Debug, Clone)]
pub struct Context {
    pub default_cwd: PathBuf,
    pub search_path: SearchPath,
}

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

impl Request {
    /// A request on the source computer `local`. `target` is the target of the
    /// handoff, or `None` for `portal-match`.
    pub fn on_source(
        local: &ComputerId,
        device: DeviceRef,
        target: Option<ComputerId>,
        displays: Vec<DisplayInfo>,
    ) -> Request {
        Request {
            computer_id: local.clone(),
            device,
            source_computer_id: local.clone(),
            target_computer_id: target,
            session_id: None,
            portal: None,
            displays,
        }
    }

    /// A request on the destination computer `local` of a handoff from
    /// `source`: `input-target` and `verify`.
    pub fn on_destination(
        local: &ComputerId,
        device: DeviceRef,
        source: ComputerId,
        displays: Vec<DisplayInfo>,
    ) -> Request {
        Request {
            computer_id: local.clone(),
            device,
            source_computer_id: source,
            target_computer_id: Some(local.clone()),
            session_id: None,
            portal: None,
            displays,
        }
    }

    /// Sets the session of the request. A hand back has no session.
    pub fn with_session(mut self, session: Option<SessionId>) -> Request {
        self.session_id = session;
        self
    }

    /// Sets the portal of the request.
    pub fn with_portal(mut self, portal: PortalInfo) -> Request {
        self.portal = Some(portal);
        self
    }
}

#[derive(Serialize)]
pub(crate) struct Envelope<'a> {
    pub(crate) protocol: u32,
    pub(crate) operation: OperationKind,
    #[serde(flatten)]
    pub(crate) request: &'a Request,
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
    #[error("{0} is not in the search path")]
    NotFound(String),
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
    context: &Context,
) -> Result<Outcome, DriverError> {
    let options = match &operation.driver {
        Driver::Exec(_) if kind == OperationKind::InputTarget => {
            return Err(DriverError::BadResult(
                "exec cannot resolve an input target".into(),
            ));
        }
        Driver::Exec(program) | Driver::Command(program) => program,
        Driver::Prompt => return Err(DriverError::Unsupported("prompt".into())),
        Driver::Extension { name, .. } => return Err(DriverError::Unsupported(name.clone())),
    };
    let mut command = program_command(options, context)?;
    if matches!(operation.driver, Driver::Exec(_)) {
        let output = run_process(
            &mut command,
            &options.argv[0],
            None,
            false,
            operation.timeout,
        )?;
        return match kind {
            OperationKind::PortalMatch | OperationKind::Verify => {
                Ok(Outcome::Matched(output.status.success()))
            }
            OperationKind::Handoff => {
                check_exit(&output)?;
                Ok(Outcome::Done)
            }
            OperationKind::InputTarget => Err(DriverError::BadResult(
                "exec cannot resolve an input target".into(),
            )),
        };
    }
    let input = serde_json::to_vec(&Envelope {
        protocol: 2,
        operation: kind,
        request,
    })
    .map_err(|e| DriverError::BadResult(e.to_string()))?;
    let output = run_process(
        &mut command,
        &options.argv[0],
        Some(&input),
        true,
        operation.timeout,
    )?;
    check_exit(&output)?;
    parse_result(kind, &output.stdout, &request.displays)
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

pub(crate) fn program_command(
    options: &Program,
    context: &Context,
) -> Result<Command, DriverError> {
    let (program, args) = options
        .argv
        .split_first()
        .ok_or_else(|| DriverError::BadResult("argv is empty".into()))?;
    let path = context
        .search_path
        .find(program)
        .ok_or_else(|| DriverError::NotFound(program.clone()))?;
    let search_path = std::env::join_paths(&context.search_path.0)
        .map_err(|e| DriverError::BadResult(format!("cannot join search path: {e}")))?;
    let mut command = Command::new(path);
    command
        .args(args)
        .current_dir(options.cwd.as_deref().unwrap_or(&context.default_cwd))
        .env("PATH", search_path)
        .envs(&options.env);
    Ok(command)
}

fn run_process(
    command: &mut Command,
    program: &str,
    input: Option<&[u8]>,
    capture_stdout: bool,
    timeout: Duration,
) -> Result<Output, DriverError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| DriverError::BadResult("timeout is too large".into()))?;
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if capture_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A new process group, so that a timeout stops the whole process tree.
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|source| DriverError::Spawn {
        program: program.to_owned(),
        source,
    })?;
    let group = ProcessGroup(child.id());

    let stdout = reader(child.stdout.take(), "stdout");
    let stderr = reader(child.stderr.take(), "stderr");
    let stdin = child.stdin.take().zip(input).map(|(mut stdin, input)| {
        let input = input.to_owned();
        thread::spawn(move || {
            // A helper can exit without reading stdin; that is not an error here.
            let _ = stdin.write_all(&input);
        })
    });

    let result = (|| {
        let status = wait(&mut child, deadline, timeout)?;
        if let Some(stdin) = stdin {
            collect(stdin, deadline, timeout)?;
        }
        Ok(Output {
            status,
            stdout: collect(stdout, deadline, timeout)??,
            stderr: collect(stderr, deadline, timeout)??,
        })
    })();
    if result.is_err() {
        group.kill();
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn check_exit(output: &Output) -> Result<(), DriverError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(DriverError::Exit {
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

fn reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    name: &'static str,
) -> thread::JoinHandle<Result<Vec<u8>, DriverError>> {
    thread::spawn(move || {
        let mut data = Vec::new();
        if let Some(pipe) = pipe {
            let mut limited = pipe.take(OUTPUT_LIMIT as u64 + 1);
            limited.read_to_end(&mut data)?;
            // Drain the rest so that the helper does not block on a full pipe.
            let mut rest = limited.into_inner();
            std::io::copy(&mut rest, &mut std::io::sink())?;
        }
        if data.len() > OUTPUT_LIMIT {
            Err(DriverError::OutputTooLarge(name))
        } else {
            Ok(data)
        }
    })
}

fn collect<T>(
    handle: thread::JoinHandle<T>,
    deadline: Instant,
    timeout: Duration,
) -> Result<T, DriverError> {
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return Err(DriverError::Timeout(timeout));
        }
        thread::sleep(Duration::from_millis(5));
    }
    handle
        .join()
        .map_err(|_| DriverError::BadResult("helper I/O thread panicked".into()))
}

fn wait(
    child: &mut Child,
    deadline: Instant,
    timeout: Duration,
) -> Result<ExitStatus, DriverError> {
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(status),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            None => return Err(DriverError::Timeout(timeout)),
        }
    }
}

pub(crate) struct ProcessGroup(pub(crate) u32);

impl ProcessGroup {
    pub(crate) fn kill(&self) {
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
