//! The verifications on this destination (VER-1 to VER-8).
//!
//! One verification checks one device on one display. It has one prompt id, one
//! verifier driver (`prompt`, `exec`, or `command`), and it sets the device
//! state (VER-6). A display shows one verification prompt (VER-8).
//!
//! A verification has one of two owners. A target session keeps the rules of
//! [`Verification`] in its `TargetSession` and calls the steps of this module for
//! its `TargetAction`s. A hand back has no session (HB-1, ACT-2): this module
//! applies the rules of [`Verification`] for it.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Instant;

use telescreen_core::config::{Driver, Operation};
use telescreen_core::driver::{DeviceRef, Request};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use telescreen_core::platform::DisplayInfo;
use telescreen_core::session::VideoInput;
use telescreen_core::verification::{Verification, VerificationAction, VerificationResult};
use telescreen_core::verifier::{PromptVerifier, VerifierEvent};
use telescreen_view::{PromptId, PromptPresenter, PromptScreen};

use super::{VerifierId, VerifierPort};

const DISPLAY_GONE: &str = "the input-target display is gone";

/// The ports and the state of the controller that the verifications use.
pub(super) trait Env {
    fn verifiers(&mut self) -> &mut dyn VerifierPort;
    fn prompt(&mut self) -> &mut dyn PromptPresenter;
    /// The local displays now.
    fn displays(&mut self) -> Vec<DisplayInfo>;
    fn notice(&mut self, text: String);
    /// The device states on this destination (VER-6).
    fn states(&mut self) -> &mut HashMap<DeviceId, VideoInput>;
}

/// The flow that owns a verification.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Owner {
    /// A target session (FLOW-7). The source acknowledges in its portal (FLOW-8).
    Session(SessionId),
    /// A hand back or "Switch display only" to this computer (HB-1, ACT-2).
    /// The user acknowledges on this destination (VER-4).
    HandBack(DeviceId),
}

/// The result of [`Verifications::start`].
#[derive(Debug, PartialEq)]
pub(super) enum Started {
    /// The device has no `verify` (VER-2).
    NoVerify,
    /// The first events of the verifier. Report each with [`Verifications::on_event`].
    Events(Vec<VerifierEvent>),
}

#[derive(Debug)]
enum Verifier {
    Prompt(PromptVerifier),
    Process(VerifierId),
}

#[derive(Debug)]
struct Entry {
    device: DeviceId,
    /// The computer that switched the display to this computer.
    source: ComputerId,
    /// The source on the prompt. `None` when the acknowledgement is on this
    /// destination (VER-4).
    label: Option<String>,
    prompt: PromptId,
    /// `None` until input-target of a hand back returns the display.
    display: Option<DisplayToken>,
    verifier: Option<Verifier>,
    /// The presenter shows the prompt.
    visible: bool,
    /// The rules of a hand back. A target session keeps its own rules.
    rules: Option<Verification>,
}

/// All verifications on this destination.
#[derive(Debug)]
pub(super) struct Verifications {
    local: ComputerId,
    confirm_key: String,
    /// The `hint` of this computer for the `prompt` verifier (VER-5).
    hint: Option<String>,
    entries: BTreeMap<Owner, Entry>,
    next_verifier: u64,
    /// Target sessions that VER-8 stopped. The controller ends each with
    /// `VerifyFailed`.
    displaced: Vec<SessionId>,
}

impl Verifications {
    pub(super) fn new(local: ComputerId, confirm_key: String, hint: Option<String>) -> Self {
        Self {
            local,
            confirm_key,
            hint,
            entries: BTreeMap::new(),
            next_verifier: 1,
            displaced: Vec::new(),
        }
    }

    /// Begins a verification of `device` for `owner`. The device state is
    /// unverified (VER-6). `display` is `None` until input-target returns it.
    #[allow(clippy::too_many_arguments)] // One field of the entry for each argument.
    pub(super) fn begin(
        &mut self,
        owner: Owner,
        device: DeviceId,
        source: ComputerId,
        label: Option<String>,
        prompt: PromptId,
        display: Option<DisplayToken>,
        env: &mut dyn Env,
    ) {
        env.states().insert(device.clone(), VideoInput::Unverified);
        let rules = matches!(owner, Owner::HandBack(_)).then(Verification::new);
        self.entries.insert(
            owner,
            Entry {
                device,
                source,
                label,
                prompt,
                display,
                verifier: None,
                visible: false,
                rules,
            },
        );
    }

    /// Sets the input-target display of a verification.
    pub(super) fn set_display(&mut self, owner: &Owner, display: DisplayToken) {
        if let Some(entry) = self.entries.get_mut(owner) {
            entry.display = Some(display);
        }
    }

    /// True while `prompt` is the prompt of the verification of `owner`. A
    /// result for an older verification does not apply.
    pub(super) fn is_current(&self, owner: &Owner, prompt: PromptId) -> bool {
        self.entries.get(owner).is_some_and(|e| e.prompt == prompt)
    }

    /// Starts the verifier of `verify` for `owner` (VER-1, VER-3, VER-5).
    pub(super) fn start(
        &mut self,
        owner: &Owner,
        alias: String,
        verify: Option<Operation>,
        now: Instant,
        env: &mut dyn Env,
    ) -> Started {
        let Some(operation) = verify else {
            return Started::NoVerify;
        };
        if !self.entries.contains_key(owner) {
            return Started::Events(Vec::new());
        }
        match &operation.driver {
            Driver::Prompt => {
                let (prompt, events) = PromptVerifier::start(self.hint.clone(), now);
                if let Some(entry) = self.entries.get_mut(owner) {
                    entry.verifier = Some(Verifier::Prompt(prompt));
                }
                Started::Events(events)
            }
            Driver::Exec(_) | Driver::Command(_) => {
                let id = VerifierId(self.next_verifier);
                self.next_verifier += 1;
                let Some(entry) = self.entries.get_mut(owner) else {
                    return Started::Events(Vec::new());
                };
                entry.verifier = Some(Verifier::Process(id));
                let session = match owner {
                    Owner::Session(session) => Some(*session),
                    Owner::HandBack(_) => None,
                };
                let request = Request::on_destination(
                    &self.local,
                    DeviceRef {
                        id: entry.device.clone(),
                        alias,
                    },
                    entry.source.clone(),
                    env.displays(),
                )
                .with_session(session);
                env.verifiers().start(id, operation, request);
                Started::Events(Vec::new())
            }
            Driver::Extension { name, .. } => Started::Events(vec![VerifierEvent::Failed(
                format!("the driver {name} is not available"),
            )]),
        }
    }

    /// The owner of the running process verifier `id`.
    pub(super) fn owner_of_verifier(&self, id: VerifierId) -> Option<Owner> {
        self.entries.iter().find_map(|(owner, entry)| {
            matches!(entry.verifier, Some(Verifier::Process(running)) if running == id)
                .then(|| owner.clone())
        })
    }

    /// Runs the timeout of each `prompt` verifier (VER-5). Report each returned
    /// event with [`Self::on_event`].
    pub(super) fn tick(&mut self, now: Instant) -> Vec<(Owner, VerifierEvent)> {
        self.entries
            .iter_mut()
            .filter_map(|(owner, entry)| match entry.verifier.as_mut() {
                Some(Verifier::Prompt(prompt)) => prompt.tick(now).map(|e| (owner.clone(), e)),
                _ => None,
            })
            .collect()
    }

    /// Reports an event of the verifier of `owner`. A final answer sets the
    /// device state (VER-6). Returns the event for a target session: its
    /// `TargetSession` applies the rules. A hand back applies the rules here.
    pub(super) fn on_event(
        &mut self,
        owner: &Owner,
        event: VerifierEvent,
        env: &mut dyn Env,
    ) -> Option<VerifierEvent> {
        self.record(owner, &event, env);
        match owner {
            Owner::Session(_) => Some(event),
            Owner::HandBack(_) => {
                let actions = self
                    .rules(owner)
                    .map(|rules| rules.on_verifier(event))
                    .unwrap_or_default();
                self.drive(owner, actions, env);
                None
            }
        }
    }

    /// Sends the acknowledgement to the verifier. The `prompt` verifier answers
    /// at once: report the returned event with [`Self::on_event`].
    pub(super) fn ack(&mut self, owner: &Owner, env: &mut dyn Env) -> Option<VerifierEvent> {
        match self.entries.get_mut(owner)?.verifier.as_mut()? {
            Verifier::Prompt(prompt) => prompt.ack(),
            Verifier::Process(id) => {
                env.verifiers().ack(*id);
                None
            }
        }
    }

    /// Stops the verifier; it sends no more events. The prompt stays.
    pub(super) fn stop_verifier(&mut self, owner: &Owner, env: &mut dyn Env) {
        let verifier = self
            .entries
            .get_mut(owner)
            .and_then(|entry| entry.verifier.take());
        if let Some(Verifier::Process(id)) = verifier {
            env.verifiers().stop(id);
        }
    }

    /// Shows the prompt on the input-target display, or updates it in place.
    /// First it stops each other verification with a visible prompt on the same
    /// display (VER-8). Returns true if the presenter shows the prompt.
    pub(super) fn show(
        &mut self,
        owner: &Owner,
        text: Option<String>,
        ack_wanted: bool,
        env: &mut dyn Env,
    ) -> bool {
        let Some(entry) = self.entries.get(owner) else {
            return false;
        };
        let prompt = entry.prompt;
        let token = entry.display.clone();
        let screen = PromptScreen {
            source: entry.label.clone(),
            confirm_key: self.confirm_key.clone(),
            text,
            ack_wanted,
            // VER-4: only a hand back has no label; a session acknowledges in
            // its portal on the source (FLOW-8).
            ack_here: ack_wanted && entry.label.is_none(),
            portal_hidden: false,
            blank: false,
        };
        let display = env
            .displays()
            .into_iter()
            .find(|d| Some(&d.token) == token.as_ref());
        if let Some(display) = &display {
            self.displace(owner, &display.token, env);
        }
        let shown = match display {
            Some(display) => env.prompt().show(prompt, &display, &screen),
            None => Err(DISPLAY_GONE.into()),
        };
        match &shown {
            Ok(()) => {
                if let Some(entry) = self.entries.get_mut(owner) {
                    entry.visible = true;
                }
            }
            Err(e) => env.notice(match owner {
                Owner::Session(_) => format!("Cannot show the prompt: {e}"),
                Owner::HandBack(_) => format!("Cannot show the hand back prompt: {e}"),
            }),
        }
        shown.is_ok()
    }

    /// Removes the prompt.
    pub(super) fn hide(&mut self, owner: &Owner, env: &mut dyn Env) {
        if let Some(entry) = self.entries.get_mut(owner) {
            env.prompt().hide(entry.prompt);
            entry.visible = false;
        }
    }

    /// Forgets the verification of a session that released its lease. The
    /// session already stopped the verifier and removed the prompt.
    pub(super) fn remove(&mut self, owner: &Owner) {
        self.entries.remove(owner);
    }

    /// Stops the verifier, removes a visible prompt, and forgets the
    /// verification. The device state does not change.
    pub(super) fn stop(&mut self, owner: &Owner, env: &mut dyn Env) {
        let Some(mut entry) = self.entries.remove(owner) else {
            return;
        };
        if let Some(Verifier::Process(id)) = entry.verifier.take() {
            env.verifiers().stop(id);
        }
        if entry.visible {
            env.prompt().hide(entry.prompt);
        }
    }

    /// The devices with a hand back verification.
    pub(super) fn hand_backs(&self) -> Vec<DeviceId> {
        self.entries
            .keys()
            .filter_map(|owner| match owner {
                Owner::HandBack(device) => Some(device.clone()),
                Owner::Session(_) => None,
            })
            .collect()
    }

    /// The window layer could not show `prompt`. The device state is unknown.
    /// Returns the session to end with `PromptFailed`; a hand back stops here.
    pub(super) fn prompt_failed(
        &mut self,
        prompt: PromptId,
        env: &mut dyn Env,
    ) -> Option<SessionId> {
        let owner = self
            .entries
            .iter()
            .find_map(|(owner, entry)| (entry.prompt == prompt).then(|| owner.clone()))?;
        env.notice("Cannot show the prompt window.".into());
        match owner {
            Owner::Session(session) => {
                if let Some(entry) = self.entries.get(&owner) {
                    env.states()
                        .insert(entry.device.clone(), VideoInput::Unknown);
                }
                Some(session)
            }
            Owner::HandBack(_) => {
                let actions = self
                    .rules(&owner)
                    .map(|rules| rules.on_shown(false))
                    .unwrap_or_default();
                self.drive(&owner, actions, env);
                None
            }
        }
    }

    /// True when a hand back verifier wants an acknowledgement on this
    /// destination (VER-4).
    pub(super) fn awaiting_local_ack(&self) -> bool {
        self.entries
            .values()
            .any(|entry| entry.rules.as_ref().is_some_and(Verification::ack_wanted))
    }

    /// Acknowledges the first hand back verifier that wants it. Returns true
    /// if a verifier accepted it.
    pub(super) fn acknowledge_local(&mut self, env: &mut dyn Env) -> bool {
        self.acknowledge_where(|_| true, env)
    }

    /// Acknowledges the hand back verifier of `prompt` (VER-4). A prompt of a
    /// target session takes no acknowledgement (FLOW-8). Returns true if the
    /// verifier accepted it.
    pub(super) fn acknowledge_prompt(&mut self, prompt: PromptId, env: &mut dyn Env) -> bool {
        self.acknowledge_where(|entry| entry.prompt == prompt, env)
    }

    fn acknowledge_where(&mut self, select: impl Fn(&Entry) -> bool, env: &mut dyn Env) -> bool {
        let owner = self.entries.iter().find_map(|(owner, entry)| {
            (select(entry) && entry.rules.as_ref().is_some_and(Verification::ack_wanted))
                .then(|| owner.clone())
        });
        let Some(owner) = owner else {
            return false;
        };
        let actions = self
            .rules(&owner)
            .map(Verification::acknowledge)
            .unwrap_or_default();
        let accepted = !actions.is_empty();
        self.drive(&owner, actions, env);
        accepted
    }

    /// The target sessions that VER-8 stopped since the last call.
    pub(super) fn take_displaced(&mut self) -> Vec<SessionId> {
        std::mem::take(&mut self.displaced)
    }

    fn rules(&mut self, owner: &Owner) -> Option<&mut Verification> {
        self.entries.get_mut(owner)?.rules.as_mut()
    }

    /// A final answer ends the verifier and sets the device state (VER-6).
    fn record(&mut self, owner: &Owner, event: &VerifierEvent, env: &mut dyn Env) {
        let Some(entry) = self.entries.get_mut(owner) else {
            return;
        };
        let state = match event {
            VerifierEvent::Done(true) => VideoInput::Confirmed,
            VerifierEvent::Done(false) | VerifierEvent::Failed(_) => VideoInput::Unknown,
            VerifierEvent::Show(_) | VerifierEvent::WantAck => return,
        };
        entry.verifier = None;
        env.states().insert(entry.device.clone(), state);
    }

    /// VER-8: stops each verification other than `owner` with a visible prompt
    /// on `display`. Its device state is unknown, except for the device of
    /// `owner`: that device keeps the state that its own verification sets.
    fn displace(&mut self, owner: &Owner, display: &DisplayToken, env: &mut dyn Env) {
        let kept = self.entries.get(owner).map(|entry| entry.device.clone());
        let others: Vec<_> = self
            .entries
            .iter()
            .filter(|(other, entry)| {
                *other != owner && entry.visible && entry.display.as_ref() == Some(display)
            })
            .map(|(other, entry)| (other.clone(), entry.device.clone()))
            .collect();
        for (other, device) in others {
            self.stop(&other, env);
            if kept.as_ref() != Some(&device) {
                env.states().insert(device, VideoInput::Unknown);
            }
            if let Owner::Session(session) = other {
                self.displaced.push(session);
            }
        }
    }

    /// Performs the actions of the rules of a hand back, in sequence.
    fn drive(&mut self, owner: &Owner, actions: Vec<VerificationAction>, env: &mut dyn Env) {
        let mut queue: VecDeque<_> = actions.into();
        while let Some(action) = queue.pop_front() {
            if !self.entries.contains_key(owner) {
                return;
            }
            let more = match action {
                VerificationAction::Show { text, ack_wanted } => {
                    let shown = self.show(owner, text, ack_wanted, env);
                    self.rules(owner).map(|rules| rules.on_shown(shown))
                }
                VerificationAction::Hide => {
                    self.hide(owner, env);
                    None
                }
                VerificationAction::Ack => match self.ack(owner, env) {
                    Some(event) => {
                        self.record(owner, &event, env);
                        self.rules(owner).map(|rules| rules.on_verifier(event))
                    }
                    None => None,
                },
                VerificationAction::Stop => {
                    self.stop_verifier(owner, env);
                    None
                }
                // The prompt shows `ack_here` (VER-4); the window layer reports the press.
                VerificationAction::AckWanted => None,
                VerificationAction::Finished(result) => {
                    self.finish(owner, result, env);
                    None
                }
            };
            for action in more.unwrap_or_default().into_iter().rev() {
                queue.push_front(action);
            }
        }
    }

    /// Ends a hand back verification. The final answer of the verifier already
    /// set the device state; a prompt that failed makes it unknown (VER-6).
    fn finish(&mut self, owner: &Owner, result: VerificationResult, env: &mut dyn Env) {
        let Some(mut entry) = self.entries.remove(owner) else {
            return;
        };
        if entry
            .rules
            .as_mut()
            .is_some_and(|rules| !rules.hide().is_empty())
        {
            env.prompt().hide(entry.prompt);
        }
        if result == VerificationResult::WindowFailed {
            env.states().insert(entry.device, VideoInput::Unknown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use telescreen_core::config::Program;

    /// Records the calls of the ports in one list.
    #[derive(Default)]
    struct FakeEnv {
        calls: Vec<String>,
        displays: Vec<DisplayInfo>,
        notices: Vec<String>,
        states: HashMap<DeviceId, VideoInput>,
        /// The next `show` fails.
        show_fails: bool,
    }

    impl VerifierPort for FakeEnv {
        fn start(&mut self, id: VerifierId, _: Operation, request: Request) {
            self.calls.push(format!(
                "start {} session={:?}",
                id.0,
                request.session_id.map(|s| s.0)
            ));
        }
        fn ack(&mut self, id: VerifierId) {
            self.calls.push(format!("ack {}", id.0));
        }
        fn stop(&mut self, id: VerifierId) {
            self.calls.push(format!("stop {}", id.0));
        }
    }

    impl PromptPresenter for FakeEnv {
        fn show(
            &mut self,
            prompt: PromptId,
            display: &DisplayInfo,
            screen: &PromptScreen,
        ) -> Result<(), String> {
            if self.show_fails {
                return Err("no window".into());
            }
            self.calls.push(format!(
                "show {} on {} ack={} here={} source={:?}",
                prompt.0, display.token.0, screen.ack_wanted, screen.ack_here, screen.source
            ));
            Ok(())
        }
        fn hide(&mut self, prompt: PromptId) {
            self.calls.push(format!("hide {}", prompt.0));
        }
    }

    impl Env for FakeEnv {
        fn verifiers(&mut self) -> &mut dyn VerifierPort {
            self
        }
        fn prompt(&mut self) -> &mut dyn PromptPresenter {
            self
        }
        fn displays(&mut self) -> Vec<DisplayInfo> {
            self.displays.clone()
        }
        fn notice(&mut self, text: String) {
            self.notices.push(text);
        }
        fn states(&mut self) -> &mut HashMap<DeviceId, VideoInput> {
            &mut self.states
        }
    }

    fn display(token: &str) -> DisplayInfo {
        DisplayInfo {
            token: DisplayToken::from(token),
            bounds: None,
            scale: 1.0,
            metadata: Default::default(),
        }
    }

    fn env() -> FakeEnv {
        FakeEnv {
            displays: vec![display("A"), display("B")],
            ..FakeEnv::default()
        }
    }

    fn verifications() -> Verifications {
        Verifications::new(
            ComputerId::from("pc1"),
            "Return".into(),
            Some("hint".into()),
        )
    }

    fn operation(driver: Driver) -> Option<Operation> {
        Some(Operation {
            driver,
            timeout: std::time::Duration::from_secs(5),
        })
    }

    fn prompt_op() -> Option<Operation> {
        operation(Driver::Prompt)
    }

    fn exec_op() -> Option<Operation> {
        operation(Driver::Exec(Program {
            argv: vec!["true".into()],
            cwd: None,
            env: Default::default(),
        }))
    }

    fn state(env: &FakeEnv, device: &str) -> Option<VideoInput> {
        env.states.get(&DeviceId::from(device)).copied()
    }

    fn session(v: &mut Verifications, env: &mut FakeEnv, id: u64, device: &str, on: &str) -> Owner {
        let owner = Owner::Session(SessionId(id));
        v.begin(
            owner.clone(),
            DeviceId::from(device),
            ComputerId::from("mac"),
            Some("Mac".into()),
            PromptId(id),
            Some(DisplayToken::from(on)),
            env,
        );
        owner
    }

    /// A hand back with a `prompt` verifier on `on`, shown and waiting for the ack.
    fn hand_back(
        v: &mut Verifications,
        env: &mut FakeEnv,
        prompt: u64,
        device: &str,
        on: &str,
    ) -> Owner {
        let owner = Owner::HandBack(DeviceId::from(device));
        v.begin(
            owner.clone(),
            DeviceId::from(device),
            ComputerId::from("mac"),
            None,
            PromptId(prompt),
            None,
            env,
        );
        v.set_display(&owner, DisplayToken::from(on));
        let Started::Events(events) =
            v.start(&owner, device.into(), prompt_op(), Instant::now(), env)
        else {
            panic!("no verify");
        };
        for event in events {
            assert_eq!(v.on_event(&owner, event, env), None);
        }
        owner
    }

    #[test]
    fn ver6_begin_sets_unverified_and_no_verify_keeps_it() {
        let mut v = verifications();
        let mut env = env();
        let owner = session(&mut v, &mut env, 1, "lg", "A");
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unverified));
        assert_eq!(
            v.start(&owner, "lg".into(), None, Instant::now(), &mut env),
            Started::NoVerify
        );
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unverified), "VER-2");
    }

    #[test]
    fn ver6_final_answers_set_the_device_state() {
        for (event, expected) in [
            (VerifierEvent::Done(true), VideoInput::Confirmed),
            (VerifierEvent::Done(false), VideoInput::Unknown),
            (VerifierEvent::Failed("x".into()), VideoInput::Unknown),
        ] {
            let mut v = verifications();
            let mut env = env();
            let owner = session(&mut v, &mut env, 1, "lg", "A");
            v.on_event(&owner, VerifierEvent::Show("text".into()), &mut env);
            assert_eq!(state(&env, "lg"), Some(VideoInput::Unverified));
            // A target session gets the event back for its own rules.
            assert_eq!(v.on_event(&owner, event.clone(), &mut env), Some(event));
            assert_eq!(state(&env, "lg"), Some(expected));
        }
    }

    #[test]
    fn ver3_a_process_verifier_gets_the_ack_and_the_stop() {
        let mut v = verifications();
        let mut env = env();
        let owner = session(&mut v, &mut env, 7, "lg", "A");
        let started = v.start(&owner, "lg".into(), exec_op(), Instant::now(), &mut env);
        assert_eq!(started, Started::Events(Vec::new()));
        assert_eq!(v.owner_of_verifier(VerifierId(1)), Some(owner.clone()));
        assert_eq!(v.ack(&owner, &mut env), None);
        v.stop_verifier(&owner, &mut env);
        assert_eq!(v.owner_of_verifier(VerifierId(1)), None);
        assert_eq!(env.calls, ["start 1 session=Some(7)", "ack 1", "stop 1"]);
    }

    #[test]
    fn ver5_a_hand_back_prompt_confirms_after_the_local_ack() {
        let mut v = verifications();
        let mut env = env();
        hand_back(&mut v, &mut env, 3, "lg", "A");
        assert!(v.awaiting_local_ack(), "VER-4");
        assert_eq!(
            env.calls.last().map(String::as_str),
            Some("show 3 on A ack=true here=true source=None")
        );
        assert!(v.acknowledge_local(&mut env));
        assert_eq!(state(&env, "lg"), Some(VideoInput::Confirmed));
        assert_eq!(env.calls.last().map(String::as_str), Some("hide 3"));
        assert!(v.hand_backs().is_empty());
        assert!(!v.acknowledge_local(&mut env));
    }

    #[test]
    fn ver4_an_acknowledgement_in_a_prompt_acts_on_that_prompt() {
        let mut v = verifications();
        let mut env = env();
        hand_back(&mut v, &mut env, 3, "lg", "A");
        hand_back(&mut v, &mut env, 4, "dell", "B");
        assert!(
            !v.acknowledge_prompt(PromptId(9), &mut env),
            "no such prompt"
        );
        assert!(v.acknowledge_prompt(PromptId(4), &mut env));
        assert_eq!(state(&env, "dell"), Some(VideoInput::Confirmed));
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unverified));
        assert_eq!(env.calls.last().map(String::as_str), Some("hide 4"));
        assert!(v.awaiting_local_ack(), "the prompt on A waits");
        assert!(!v.acknowledge_prompt(PromptId(4), &mut env), "one ack");
        assert!(v.acknowledge_prompt(PromptId(3), &mut env));
        assert_eq!(state(&env, "lg"), Some(VideoInput::Confirmed));
        assert!(!v.awaiting_local_ack());
    }

    #[test]
    fn ver4_a_session_prompt_takes_no_local_acknowledgement() {
        let mut v = verifications();
        let mut env = env();
        let owner = session(&mut v, &mut env, 2, "lg", "A");
        assert!(v.show(&owner, None, true, &mut env));
        assert!(!v.acknowledge_prompt(PromptId(2), &mut env), "FLOW-8");
        assert!(!v.acknowledge_local(&mut env));
    }

    #[test]
    fn ver5_a_hand_back_prompt_times_out_as_unknown() {
        let mut v = verifications();
        let mut env = env();
        let owner = hand_back(&mut v, &mut env, 3, "lg", "A");
        let later = Instant::now() + std::time::Duration::from_secs(61);
        let events = v.tick(later);
        assert_eq!(events.len(), 1);
        for (o, event) in events {
            assert_eq!(o, owner);
            v.on_event(&o, event, &mut env);
        }
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unknown), "FLOW-11");
        assert_eq!(env.calls.last().map(String::as_str), Some("hide 3"));
        assert!(!v.awaiting_local_ack());
    }

    #[test]
    fn ver6_a_hand_back_prompt_that_cannot_show_is_unknown() {
        let mut v = verifications();
        let mut env = env();
        env.show_fails = true;
        hand_back(&mut v, &mut env, 3, "lg", "A");
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unknown));
        assert!(v.hand_backs().is_empty());
        assert_eq!(env.notices, ["Cannot show the hand back prompt: no window"]);
    }

    #[test]
    fn ver6_a_failed_prompt_window_is_unknown_for_both_owners() {
        let mut v = verifications();
        let mut env = env();
        let owner = session(&mut v, &mut env, 1, "lg", "A");
        assert!(v.show(&owner, None, true, &mut env));
        assert_eq!(v.prompt_failed(PromptId(1), &mut env), Some(SessionId(1)));
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unknown));
        hand_back(&mut v, &mut env, 2, "dell", "B");
        assert_eq!(v.prompt_failed(PromptId(2), &mut env), None);
        assert_eq!(state(&env, "dell"), Some(VideoInput::Unknown));
        assert!(v.hand_backs().is_empty());
    }

    #[test]
    fn ver8_a_new_session_prompt_stops_an_earlier_session_prompt() {
        let mut v = verifications();
        let mut env = env();
        let first = session(&mut v, &mut env, 1, "lg", "A");
        v.start(&first, "lg".into(), exec_op(), Instant::now(), &mut env);
        assert!(v.show(&first, None, true, &mut env));
        let second = session(&mut v, &mut env, 2, "dell", "A");
        env.calls.clear();
        assert!(v.show(&second, None, true, &mut env));
        // The earlier prompt goes away before the new one shows.
        assert_eq!(
            env.calls,
            [
                "stop 1",
                "hide 1",
                "show 2 on A ack=true here=false source=Some(\"Mac\")"
            ]
        );
        assert_eq!(v.take_displaced(), [SessionId(1)]);
        assert!(v.take_displaced().is_empty());
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unknown));
        assert_eq!(state(&env, "dell"), Some(VideoInput::Unverified));
    }

    #[test]
    fn ver8_a_session_prompt_stops_a_hand_back_prompt_on_the_display() {
        let mut v = verifications();
        let mut env = env();
        hand_back(&mut v, &mut env, 1, "lg", "A");
        let owner = session(&mut v, &mut env, 2, "dell", "A");
        assert!(v.show(&owner, None, true, &mut env));
        assert!(v.hand_backs().is_empty());
        assert!(!v.awaiting_local_ack());
        assert!(v.take_displaced().is_empty(), "a hand back stops here");
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unknown));
    }

    #[test]
    fn ver8_the_same_device_keeps_the_state_of_its_new_verification() {
        let mut v = verifications();
        let mut env = env();
        let first = session(&mut v, &mut env, 1, "lg", "A");
        assert!(v.show(&first, None, true, &mut env));
        // The new session of the same device starts with an unverified state.
        let second = session(&mut v, &mut env, 2, "lg", "A");
        assert!(v.show(&second, None, true, &mut env));
        assert_eq!(v.take_displaced(), [SessionId(1)]);
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unverified));
    }

    #[test]
    fn ver8_other_displays_and_hidden_prompts_stay() {
        let mut v = verifications();
        let mut env = env();
        let on_b = session(&mut v, &mut env, 1, "lg", "B");
        assert!(v.show(&on_b, None, true, &mut env));
        // A finished verification hid its prompt on A.
        let hidden = session(&mut v, &mut env, 2, "dell", "A");
        assert!(v.show(&hidden, None, true, &mut env));
        v.hide(&hidden, &mut env);
        let owner = hand_back(&mut v, &mut env, 3, "tv", "A");
        assert!(v.take_displaced().is_empty());
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unverified));
        assert_eq!(state(&env, "dell"), Some(VideoInput::Unverified));
        assert_eq!(v.hand_backs(), [DeviceId::from("tv")]);
        assert!(v.is_current(&owner, PromptId(3)));
    }

    #[test]
    fn a_prompt_on_a_display_that_is_gone_does_not_show() {
        let mut v = verifications();
        let mut env = env();
        let owner = session(&mut v, &mut env, 1, "lg", "gone");
        assert!(!v.show(&owner, None, false, &mut env));
        assert_eq!(
            env.notices,
            [format!("Cannot show the prompt: {DISPLAY_GONE}")]
        );
        assert!(env.calls.is_empty());
    }

    #[test]
    fn stop_hides_a_visible_prompt_and_keeps_the_state() {
        let mut v = verifications();
        let mut env = env();
        let owner = hand_back(&mut v, &mut env, 4, "lg", "A");
        env.calls.clear();
        v.stop(&owner, &mut env);
        assert_eq!(env.calls, ["hide 4"]);
        assert_eq!(state(&env, "lg"), Some(VideoInput::Unverified));
        assert!(!v.is_current(&owner, PromptId(4)));
    }
}
