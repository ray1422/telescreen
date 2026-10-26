//! The ports of the controller that do not depend on the OS: the network and the
//! driver job runner.

use std::path::PathBuf;
use std::rc::Rc;
use std::thread;

use telescreen_transport::{NetCommand, Network};
use winit::event_loop::EventLoopProxy;

use super::UserEvent;
use crate::controller::{Job, JobId, JobPort, NetPort, run_job};

pub struct NetHandle(pub Rc<Network>);

impl NetPort for NetHandle {
    fn send(&mut self, command: NetCommand) {
        self.0.send(command);
    }
}

/// Runs each driver job on its own thread (`docs/config.md`: never on the input
/// path or the UI thread), and sends the result to the event loop.
pub struct ThreadJobs {
    pub proxy: EventLoopProxy<UserEvent>,
    pub cwd: PathBuf,
}

impl JobPort for ThreadJobs {
    fn spawn(&mut self, id: JobId, job: Job) {
        let proxy = self.proxy.clone();
        let cwd = self.cwd.clone();
        let started = thread::Builder::new()
            .name("telescreen-driver".into())
            .spawn(move || {
                let output = run_job(&job, &cwd);
                let _ = proxy.send_event(UserEvent::Job(id, output));
            });
        if let Err(e) = started {
            let output =
                crate::controller::JobOutput::Operation(Err(format!("cannot start a thread: {e}")));
            let _ = self.proxy.send_event(UserEvent::Job(id, output));
        }
    }
}
