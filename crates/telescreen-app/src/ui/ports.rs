//! The ports of the controller that do not depend on the OS: the network and the
//! driver job runner and verifier runtime.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

use telescreen_core::config::{Driver, Operation};
use telescreen_core::driver::{self, DriverError, OperationKind, Outcome, Request};
use telescreen_core::verifier::{VerifierEvent, VerifierProcess};
use telescreen_transport::{NetCommand, Network};
use winit::event_loop::EventLoopProxy;

use super::UserEvent;
use crate::controller::{Job, JobId, JobPort, NetPort, VerifierId, VerifierPort, run_job};

pub struct NetHandle(pub Option<Rc<Network>>);

impl NetPort for NetHandle {
    fn send(&mut self, command: NetCommand) {
        if let Some(network) = &self.0 {
            network.send(command);
        }
    }

    fn shutdown(&mut self) {
        drop(self.0.take());
    }
}

/// Runs each driver job on its own thread (`docs/config.md`: never on the input
/// path or the UI thread), and sends the result to the event loop.
pub struct ThreadJobs {
    pub proxy: EventLoopProxy<UserEvent>,
    pub context: driver::Context,
}

impl JobPort for ThreadJobs {
    fn spawn(&mut self, id: JobId, job: Job) {
        let proxy = self.proxy.clone();
        let context = self.context.clone();
        let started = thread::Builder::new()
            .name("telescreen-driver".into())
            .spawn(move || {
                let output = run_job(&job, &context);
                let _ = proxy.send_event(UserEvent::Job(id, output));
            });
        if let Err(e) = started {
            let output =
                crate::controller::JobOutput::Operation(Err(format!("cannot start a thread: {e}")));
            let _ = self.proxy.send_event(UserEvent::Job(id, output));
        }
    }
}

#[derive(Default)]
pub struct RunningVerifier {
    process: Option<VerifierProcess>,
    acknowledgements: usize,
}

/// Runs verifier startup and execution on workers. Reservations let stop and
/// acknowledgement race safely with process startup without blocking the UI.
pub struct ThreadVerifiers {
    pub proxy: EventLoopProxy<UserEvent>,
    pub context: driver::Context,
    pub running: Arc<Mutex<HashMap<VerifierId, RunningVerifier>>>,
}

impl VerifierPort for ThreadVerifiers {
    fn start(&mut self, id: VerifierId, operation: Operation, request: Request) {
        let proxy = self.proxy.clone();
        let context = self.context.clone();
        let running = Arc::clone(&self.running);
        if matches!(operation.driver, Driver::Command(_)) {
            running
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, RunningVerifier::default());
        }
        let started = thread::Builder::new()
            .name("telescreen-verifier".into())
            .spawn(move || {
                let event = match &operation.driver {
                    Driver::Exec(_) => {
                        match driver::run(&operation, OperationKind::Verify, &request, &context) {
                            Ok(Outcome::Matched(ok)) => VerifierEvent::Done(ok),
                            Ok(other) => VerifierEvent::Failed(
                                DriverError::BadResult(format!("verify gave {other:?}"))
                                    .to_string(),
                            ),
                            Err(error) => VerifierEvent::Failed(error.to_string()),
                        }
                    }
                    Driver::Command(program) => {
                        let events = proxy.clone();
                        let finished = Arc::clone(&running);
                        match VerifierProcess::spawn(
                            program,
                            &request,
                            &context,
                            operation.timeout,
                            move |event| {
                                if matches!(
                                    event,
                                    VerifierEvent::Done(_) | VerifierEvent::Failed(_)
                                ) {
                                    finished
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .remove(&id);
                                }
                                let _ = events.send_event(UserEvent::Verifier(id, event));
                            },
                        ) {
                            Ok(process) => {
                                let mut running = running.lock().unwrap_or_else(|e| e.into_inner());
                                if let Some(entry) = running.get_mut(&id) {
                                    for _ in 0..entry.acknowledgements {
                                        process.ack();
                                    }
                                    entry.process = Some(process);
                                } else {
                                    process.stop();
                                }
                                return;
                            }
                            Err(error) => {
                                running
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&id);
                                VerifierEvent::Failed(error.to_string())
                            }
                        }
                    }
                    Driver::Prompt => {
                        VerifierEvent::Failed(DriverError::Unsupported("prompt".into()).to_string())
                    }
                    Driver::Display { .. } => VerifierEvent::Failed(
                        DriverError::Unsupported("display".into()).to_string(),
                    ),
                    Driver::Extension { name, .. } => {
                        VerifierEvent::Failed(DriverError::Unsupported(name.clone()).to_string())
                    }
                };
                let _ = proxy.send_event(UserEvent::Verifier(id, event));
            });
        if let Err(error) = started {
            self.running
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            let _ = self.proxy.send_event(UserEvent::Verifier(
                id,
                VerifierEvent::Failed(format!("cannot start a thread: {error}")),
            ));
        }
    }

    fn ack(&mut self, id: VerifierId) {
        if let Some(entry) = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&id)
        {
            if let Some(process) = &entry.process {
                process.ack();
            } else {
                entry.acknowledgements += 1;
            }
        }
    }

    fn stop(&mut self, id: VerifierId) {
        let entry = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if let Some(process) = entry.and_then(|entry| entry.process) {
            process.stop();
        }
    }
}
