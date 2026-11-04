//! The JSON-lines verifier runtime and the built-in prompt (VER-3 and VER-5).

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::Program;
use crate::driver::{
    Context, DriverError, Envelope, OUTPUT_LIMIT, OperationKind, ProcessGroup, Request,
    program_command,
};

/// The fixed time allowed for an acknowledgement of the built-in prompt.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifierLine {
    Show(String),
    WantAck,
    Result(bool),
}

/// Parses exactly one verifier object, including its value and text limits.
pub fn parse_line(line: &str) -> Result<VerifierLine, String> {
    // Externally tagged enums reject additional fields, including duplicates.
    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Line {
        Show(String),
        Want(String),
        Ok(bool),
    }

    match serde_json::from_str(line).map_err(|error| error.to_string())? {
        Line::Show(text) if (1..=500).contains(&text.chars().count()) => {
            Ok(VerifierLine::Show(text))
        }
        Line::Show(_) => Err("show must contain 1 to 500 characters".into()),
        Line::Want(want) if want == "ack" => Ok(VerifierLine::WantAck),
        Line::Want(_) => Err("want must be the string ack".into()),
        Line::Ok(ok) => Ok(VerifierLine::Result(ok)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifierEvent {
    Show(String),
    WantAck,
    Done(bool),
    Failed(String),
}

enum Input {
    Ack,
    Close,
}

enum Message {
    Line(VerifierLine),
    StdoutClosed,
    StderrClosed,
    StdinClosed,
    Failed(DriverError),
}

/// A verifier whose pipe I/O and lifecycle run on worker threads.
pub struct VerifierProcess {
    input: Sender<Input>,
    stopped: Arc<AtomicBool>,
    group: ProcessGroup,
}

impl std::fmt::Debug for VerifierProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifierProcess")
            .field("process_group", &self.group.0)
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

struct Runtime {
    messages: Receiver<Message>,
    input: Sender<Input>,
    input_closed: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    deadline: Instant,
    timeout: Duration,
}

impl VerifierProcess {
    pub fn spawn(
        program: &Program,
        request: &Request,
        context: &Context,
        timeout: Duration,
        on_event: impl Fn(VerifierEvent) + Send + 'static,
    ) -> Result<VerifierProcess, DriverError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| DriverError::BadResult("timeout is too large".into()))?;
        let mut command = program_command(program, context)?;
        let mut request = serde_json::to_vec(&Envelope {
            protocol: 2,
            operation: OperationKind::Verify,
            request,
        })
        .map_err(|error| DriverError::BadResult(error.to_string()))?;
        request.push(b'\n');
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|source| DriverError::Spawn {
            program: program.argv.first().cloned().unwrap_or_default(),
            source,
        })?;
        let group = ProcessGroup(child.id());
        let stopped = Arc::new(AtomicBool::new(false));
        let input_closed = Arc::new(AtomicBool::new(false));
        let (input, inputs) = mpsc::channel();
        let (messages, received) = mpsc::channel();

        let stdout = child.stdout.take();
        let output_messages = messages.clone();
        thread::spawn(move || {
            let result = read_stdout(stdout, &output_messages);
            let message = match result {
                Ok(()) => Message::StdoutClosed,
                Err(error) => Message::Failed(error),
            };
            let _ = output_messages.send(message);
        });

        let stderr = child.stderr.take();
        let error_messages = messages.clone();
        thread::spawn(move || {
            let result = read_stderr(stderr);
            let message = match result {
                Ok(()) => Message::StderrClosed,
                Err(error) => Message::Failed(error),
            };
            let _ = error_messages.send(message);
        });

        let stdin = child.stdin.take();
        let writer_closed = Arc::clone(&input_closed);
        thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                // A verifier can ignore the request and close stdin, as with
                // the `command` driver.
                let _ = stdin.write_all(&request);
                while let Ok(Input::Ack) = inputs.recv() {
                    if writer_closed.load(Ordering::SeqCst) {
                        break;
                    }
                    // An acknowledgement may race with the verifier exiting.
                    let _ = stdin.write_all(b"{\"ack\":true}\n");
                }
            }
            let _ = messages.send(Message::StdinClosed);
        });

        let process = VerifierProcess {
            input,
            stopped,
            group,
        };
        let runtime = Runtime {
            messages: received,
            input: process.input.clone(),
            input_closed,
            stopped: Arc::clone(&process.stopped),
            deadline,
            timeout,
        };
        thread::spawn(move || {
            let group = ProcessGroup(child.id());
            let result = runtime.supervise(&mut child, &on_event);
            runtime.input_closed.store(true, Ordering::SeqCst);
            let _ = runtime.input.send(Input::Close);
            if result.is_err() || runtime.stopped.load(Ordering::SeqCst) {
                group.kill();
                let _ = child.kill();
            }
            // Reap on this worker, never on the thread calling stop.
            let _ = child.wait();
            if !runtime.stopped.load(Ordering::SeqCst) {
                on_event(match result {
                    Ok(ok) => VerifierEvent::Done(ok),
                    Err(error) => VerifierEvent::Failed(error.to_string()),
                });
            }
        });
        Ok(process)
    }

    /// Queues an acknowledgement without waiting for the stdin pipe.
    pub fn ack(&self) {
        let _ = self.input.send(Input::Ack);
    }

    /// Suppresses further events and stops the verifier's entire process group.
    pub fn stop(self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.group.kill();
        let _ = self.input.send(Input::Close);
    }
}

fn read_stdout(stdout: Option<impl Read>, messages: &Sender<Message>) -> Result<(), DriverError> {
    let Some(stdout) = stdout else {
        return Err(DriverError::BadResult("stdout pipe is missing".into()));
    };
    let mut reader = BufReader::new(stdout.take(OUTPUT_LIMIT as u64 + 1));
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let count = reader.read_until(b'\n', &mut line)?;
        total += count;
        if total > OUTPUT_LIMIT {
            return Err(DriverError::OutputTooLarge("stdout"));
        }
        if count == 0 {
            return Ok(());
        }
        if line.pop() != Some(b'\n') {
            return Err(DriverError::BadResult(
                "verifier line must end with a newline".into(),
            ));
        }
        let text = std::str::from_utf8(&line)
            .map_err(|error| DriverError::BadResult(format!("stdout is not UTF-8: {error}")))?;
        let line = parse_line(text).map_err(DriverError::BadResult)?;
        if messages.send(Message::Line(line)).is_err() {
            return Ok(());
        }
    }
}

fn read_stderr(stderr: Option<impl Read>) -> Result<(), DriverError> {
    let Some(stderr) = stderr else {
        return Err(DriverError::BadResult("stderr pipe is missing".into()));
    };
    let count = std::io::copy(
        &mut stderr.take(OUTPUT_LIMIT as u64 + 1),
        &mut std::io::sink(),
    )?;
    if count > OUTPUT_LIMIT as u64 {
        Err(DriverError::OutputTooLarge("stderr"))
    } else {
        Ok(())
    }
}

impl Runtime {
    fn supervise(
        &self,
        child: &mut Child,
        on_event: &impl Fn(VerifierEvent),
    ) -> Result<bool, DriverError> {
        let mut result = None;
        let mut stdout_closed = false;
        let mut stderr_closed = false;
        let mut stdin_closed = false;
        loop {
            if self.stopped.load(Ordering::SeqCst) {
                // The worker suppresses this value when stopped.
                return Ok(false);
            }
            let now = Instant::now();
            if now >= self.deadline {
                return Err(DriverError::Timeout(self.timeout));
            }
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(DriverError::Exit {
                        status: status.to_string(),
                        stderr: String::new(),
                    });
                }
                if stdout_closed && stderr_closed && stdin_closed {
                    return result.ok_or_else(|| {
                        DriverError::BadResult("verifier exited without a result".into())
                    });
                }
            }
            let message = match self
                .messages
                .recv_timeout((self.deadline - now).min(Duration::from_millis(5)))
            {
                Ok(message) => message,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if stdout_closed && stderr_closed && stdin_closed {
                        // All pipes are closed; wait for the exit status.
                        thread::sleep((self.deadline - now).min(Duration::from_millis(5)));
                        continue;
                    }
                    return Err(DriverError::BadResult(
                        "verifier I/O threads stopped".into(),
                    ));
                }
            };
            if self.stopped.load(Ordering::SeqCst) {
                return Ok(false);
            }
            match message {
                Message::Line(_) if result.is_some() => {
                    return Err(DriverError::BadResult("line after verifier result".into()));
                }
                Message::Line(VerifierLine::Show(text)) => on_event(VerifierEvent::Show(text)),
                Message::Line(VerifierLine::WantAck) => on_event(VerifierEvent::WantAck),
                Message::Line(VerifierLine::Result(ok)) => {
                    result = Some(ok);
                    self.input_closed.store(true, Ordering::SeqCst);
                    let _ = self.input.send(Input::Close);
                }
                Message::StdoutClosed => {
                    stdout_closed = true;
                    if result.is_none() {
                        return Err(DriverError::BadResult("verifier has no result".into()));
                    }
                }
                Message::StderrClosed => stderr_closed = true,
                Message::StdinClosed => stdin_closed = true,
                Message::Failed(error) => return Err(error),
            }
        }
    }
}

/// The built-in prompt, driven by acknowledgement and clock events.
#[derive(Debug)]
pub struct PromptVerifier {
    start: Instant,
    finished: bool,
}

impl PromptVerifier {
    pub fn start(hint: Option<String>, now: Instant) -> (PromptVerifier, Vec<VerifierEvent>) {
        let mut events = Vec::new();
        if let Some(hint) = hint {
            events.push(VerifierEvent::Show(hint));
        }
        events.push(VerifierEvent::WantAck);
        (
            PromptVerifier {
                start: now,
                finished: false,
            },
            events,
        )
    }

    pub fn ack(&mut self) -> Option<VerifierEvent> {
        self.finish(true)
    }

    pub fn tick(&mut self, now: Instant) -> Option<VerifierEvent> {
        if now.saturating_duration_since(self.start) >= PROMPT_TIMEOUT {
            self.finish(false)
        } else {
            None
        }
    }

    fn finish(&mut self, ok: bool) -> Option<VerifierEvent> {
        if self.finished {
            None
        } else {
            self.finished = true;
            Some(VerifierEvent::Done(ok))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_require_one_field_and_count_unicode_characters() {
        for line in [
            r#"{"ok":true,"ok":false}"#,
            r#"{"show":"first","show":"second"}"#,
            r#"{"want":{"ack":null}}"#,
            r#"{"want":null}"#,
            r#"{"ok":true} {"ok":false}"#,
            r#""ack""#,
            "null",
        ] {
            assert!(parse_line(line).is_err(), "{line}");
        }
        let text = "界".repeat(500);
        assert_eq!(
            parse_line(&format!(r#"{{"show":"{text}"}}"#)),
            Ok(VerifierLine::Show(text))
        );
        assert!(parse_line(&format!(r#"{{"show":"{}"}}"#, "界".repeat(501))).is_err());
    }

    #[cfg(unix)]
    mod process {
        use super::*;
        use crate::driver::{DeviceRef, SearchPath};

        type TestResult = Result<(), Box<dyn std::error::Error>>;

        fn request(alias: String) -> Request {
            Request {
                computer_id: "destination".into(),
                device: DeviceRef {
                    id: "device".into(),
                    alias,
                },
                source_computer_id: "source".into(),
                target_computer_id: Some("destination".into()),
                session_id: None,
                portal: None,
                displays: Vec::new(),
            }
        }

        fn start(
            body: &str,
            request: &Request,
            timeout: Duration,
        ) -> Result<(VerifierProcess, Receiver<VerifierEvent>), DriverError> {
            let program = Program {
                argv: vec!["/bin/sh".into(), "-c".into(), body.into()],
                cwd: None,
                env: Default::default(),
            };
            let context = Context {
                default_cwd: std::env::temp_dir(),
                search_path: SearchPath::from_env(),
            };
            let (events, received) = mpsc::channel();
            let process =
                VerifierProcess::spawn(&program, request, &context, timeout, move |event| {
                    let _ = events.send(event);
                })?;
            Ok((process, received))
        }

        #[test]
        fn result_closes_stdin_before_exit_and_finishes_once() -> TestResult {
            let (_process, events) = start(
                r#"read -r request; echo '{"ok":true}'; if read -r ack; then exit 8; fi"#,
                &request("Display".into()),
                Duration::from_secs(5),
            )?;
            assert_eq!(
                events.recv_timeout(Duration::from_secs(5))?,
                VerifierEvent::Done(true)
            );
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(1)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ));
            Ok(())
        }

        #[test]
        fn invalid_output_and_output_limits_fail_once() -> TestResult {
            for body in [
                r#"read -r request; echo '{"ok":true}'; echo '{"show":"late"}'"#,
                r#"read -r request; printf '{"ok":true}'"#,
                "read -r request; printf '%65537s' ''; sleep 30",
                "read -r request; printf '%65537s' '' >&2; sleep 30",
            ] {
                let (_process, events) =
                    start(body, &request("Display".into()), Duration::from_secs(5))?;
                assert!(matches!(
                    events.recv_timeout(Duration::from_secs(5))?,
                    VerifierEvent::Failed(_)
                ));
                assert!(matches!(
                    events.recv_timeout(Duration::from_secs(1)),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                ));
            }
            Ok(())
        }

        #[test]
        fn timeout_still_applies_after_result() -> TestResult {
            let (_process, events) = start(
                r#"read -r request; echo '{"ok":true}'; exec sleep 30"#,
                &request("Display".into()),
                Duration::from_millis(200),
            )?;
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(5))?,
                VerifierEvent::Failed(_)
            ));
            Ok(())
        }

        #[test]
        fn blocked_stdin_does_not_block_spawn_ack_or_stop() -> TestResult {
            // The request exceeds pipe capacity, and the verifier never reads it.
            let now = Instant::now();
            let (process, events) = start(
                r#"echo '{"want":"ack"}'; exec sleep 30"#,
                &request("x".repeat(1024 * 1024)),
                Duration::from_secs(5),
            )?;
            assert!(now.elapsed() < Duration::from_secs(2));
            assert_eq!(
                events.recv_timeout(Duration::from_secs(5))?,
                VerifierEvent::WantAck
            );
            let now = Instant::now();
            process.ack();
            process.stop();
            assert!(now.elapsed() < Duration::from_secs(1));
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(2)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ));
            Ok(())
        }
    }
}
