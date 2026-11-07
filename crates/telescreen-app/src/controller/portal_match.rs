//! The `portal-match` evaluations of this computer (FLOW-2, FLOW-4, AC-9).
//!
//! One evaluation runs `portal-match` for a set of candidates. A candidate is
//! one local device on one display. Three flows start evaluations: a portal
//! (FLOW-2), the menu for each display (UI-18), and a received control request
//! (ACT-4). This module makes the jobs, drops the results of an older
//! evaluation of the same owner (FLOW-4), and folds the answers into one
//! [`MatchResult`]. The rule of AC-9 is in [`Evaluation::result`].

use std::collections::HashMap;

use telescreen_core::config::Device;
use telescreen_core::driver::{DeviceRef, OperationKind, Outcome, PortalInfo, Request};
use telescreen_core::ids::{ComputerId, DisplayToken, PortalId};
use telescreen_core::platform::DisplayInfo;

use super::{Job, JobOutput};

/// The flow that owns an evaluation. An owner has at most one evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum MatchOwner {
    /// The devices on the display of a portal (FLOW-2).
    Portal(PortalId),
    /// The devices on one display, for the menu (UI-18).
    Display(DisplayToken),
    /// One device on each display, for a received control request (ACT-4).
    Control(u64),
}

/// One local device on one display.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Candidate {
    /// The local alias of the device.
    pub(super) alias: String,
    pub(super) device: Device,
    pub(super) display: DisplayInfo,
}

/// Identifies the job of one candidate. A [`Pending`](super::Pending) keeps it.
#[derive(Debug, Clone)]
pub(super) struct Ticket {
    owner: MatchOwner,
    generation: u64,
    index: usize,
}

/// The answer of `portal-match` for one candidate.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Answer {
    Matched(bool),
    /// The driver failed, or the output is not a match result. The text has no alias.
    Failed(String),
}

impl Answer {
    pub(super) fn from_output(output: &JobOutput) -> Self {
        match output {
            JobOutput::Operation(Ok(Outcome::Matched(matched))) => Self::Matched(*matched),
            JobOutput::Operation(Err(e)) => Self::Failed(e.clone()),
            other => Self::Failed(format!("unexpected {other:?}")),
        }
    }
}

/// The result of a finished evaluation (AC-9).
#[derive(Debug, PartialEq)]
pub(super) enum MatchResult<'a> {
    /// At least one candidate failed. The text is the last error, with its alias.
    Failed(&'a str),
    NoMatch,
    One(&'a Candidate),
    /// More than one candidate matches, in the sequence of their answers.
    Many(Vec<&'a Candidate>),
}

/// The answers of one evaluation.
#[derive(Debug)]
pub(super) struct Evaluation {
    candidates: Vec<Candidate>,
    pending: usize,
    /// The candidates that answered.
    answered: Vec<bool>,
    /// The indices of the matching candidates, in the sequence of their answers.
    matched: Vec<usize>,
    /// The last error, as "alias: error".
    error: Option<String>,
}

impl Evaluation {
    pub(super) fn new(candidates: Vec<Candidate>) -> Self {
        Self {
            pending: candidates.len(),
            answered: vec![false; candidates.len()],
            candidates,
            matched: Vec::new(),
            error: None,
        }
    }

    /// Records the answer of one candidate. A second answer of a candidate
    /// does not apply.
    pub(super) fn record(&mut self, index: usize, answer: Answer) {
        let Some(candidate) = self.candidates.get(index) else {
            return;
        };
        if std::mem::replace(&mut self.answered[index], true) {
            return;
        }
        self.pending -= 1;
        match answer {
            Answer::Matched(true) => self.matched.push(index),
            Answer::Matched(false) => {}
            Answer::Failed(e) => self.error = Some(format!("{}: {e}", candidate.alias)),
        }
    }

    /// True while a candidate has no answer.
    pub(super) fn is_pending(&self) -> bool {
        self.pending > 0
    }

    /// The last error, also while other candidates have no answer.
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The matching candidates, in the sequence of their answers.
    pub(super) fn matched(&self) -> impl Iterator<Item = &Candidate> {
        self.matched.iter().map(|i| &self.candidates[*i])
    }

    /// True if the evaluation finished with no error and the device is one of
    /// the matches. An action that names the device uses it (ACT-6): then
    /// another matching device does not prevent the start.
    pub(super) fn found(&self, alias: &str) -> bool {
        !self.is_pending() && self.error.is_none() && self.matched().any(|c| c.alias == alias)
    }

    /// `None` while a candidate has no answer. AC-9: an error, no match, or
    /// more than one match never selects a candidate.
    pub(super) fn result(&self) -> Option<MatchResult<'_>> {
        if self.is_pending() {
            return None;
        }
        if let Some(error) = &self.error {
            return Some(MatchResult::Failed(error));
        }
        let mut matched: Vec<_> = self.matched().collect();
        Some(match matched.len() {
            0 => MatchResult::NoMatch,
            1 => MatchResult::One(matched.remove(0)),
            _ => MatchResult::Many(matched),
        })
    }
}

/// The evaluations of all owners.
#[derive(Debug, Default)]
pub(super) struct PortalMatches {
    next_generation: u64,
    runs: HashMap<MatchOwner, (u64, Evaluation)>,
}

impl PortalMatches {
    /// Starts a new evaluation for the owner. It replaces the old evaluation of
    /// the owner: the old jobs do not apply (FLOW-4). A candidate with no
    /// `portal-match` gets no job and is not part of the evaluation. The caller
    /// spawns the returned jobs and gives each result to [`Self::record`].
    ///
    /// `portal_id` is the `portal.id` of the requests (docs/config.md).
    pub(super) fn start(
        &mut self,
        owner: MatchOwner,
        portal_id: &str,
        candidates: Vec<Candidate>,
        local: &ComputerId,
        displays: &[DisplayInfo],
    ) -> Vec<(Ticket, Job)> {
        let generation = self.next_generation;
        self.next_generation += 1;
        let candidates: Vec<_> = candidates
            .into_iter()
            .filter(|c| c.device.portal_match.is_some())
            .collect();
        let jobs = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                let operation = candidate.device.portal_match.clone()?;
                let request = Request::on_source(
                    local,
                    DeviceRef {
                        id: candidate.device.id.clone(),
                        alias: candidate.alias.clone(),
                    },
                    None,
                    displays.to_vec(),
                )
                .with_portal(portal_info(portal_id.to_owned(), &candidate.display));
                let ticket = Ticket {
                    owner: owner.clone(),
                    generation,
                    index,
                };
                let job = Job::Operation {
                    operation,
                    kind: OperationKind::PortalMatch,
                    request,
                };
                Some((ticket, job))
            })
            .collect();
        self.runs
            .insert(owner, (generation, Evaluation::new(candidates)));
        jobs
    }

    /// Records the result of one job. Returns the owner, or `None` if the
    /// evaluation of the job is gone or was replaced (FLOW-4).
    pub(super) fn record(&mut self, ticket: Ticket, output: &JobOutput) -> Option<MatchOwner> {
        let (generation, evaluation) = self.runs.get_mut(&ticket.owner)?;
        if *generation != ticket.generation {
            return None;
        }
        evaluation.record(ticket.index, Answer::from_output(output));
        Some(ticket.owner)
    }

    pub(super) fn get(&self, owner: &MatchOwner) -> Option<&Evaluation> {
        self.runs.get(owner).map(|(_, e)| e)
    }

    /// Removes the evaluation of the owner. Its running jobs do not apply.
    pub(super) fn remove(&mut self, owner: &MatchOwner) -> Option<Evaluation> {
        self.runs.remove(owner).map(|(_, e)| e)
    }

    /// Keeps only the evaluations of the owners that `keep` accepts.
    pub(super) fn retain(&mut self, mut keep: impl FnMut(&MatchOwner) -> bool) {
        self.runs.retain(|owner, _| keep(owner));
    }
}

/// The `portal` of a request for an operation on a display (docs/config.md).
pub(super) fn portal_info(id: String, display: &DisplayInfo) -> PortalInfo {
    PortalInfo {
        id,
        bounds: display.bounds,
        coordinate_space: "local-desktop-logical",
        display_token: Some(display.token.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use telescreen_core::config::{Driver, Operation, Program};
    use telescreen_core::ids::DeviceId;
    use telescreen_core::pointer::Rect;

    fn display(token: &str) -> DisplayInfo {
        DisplayInfo {
            token: DisplayToken::from(token),
            bounds: Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            }),
            scale: 1.0,
            metadata: Default::default(),
        }
    }

    fn device(id: &str, portal_match: bool) -> Device {
        Device {
            id: DeviceId::from(id),
            label: None,
            portal_match: portal_match.then(|| Operation {
                driver: Driver::Exec(Program {
                    argv: vec!["match".into()],
                    cwd: None,
                    env: Default::default(),
                }),
                timeout: std::time::Duration::from_secs(5),
            }),
            input_target: None,
            verify: None,
            handoff: Default::default(),
        }
    }

    fn candidate(alias: &str, on: &str) -> Candidate {
        Candidate {
            alias: alias.into(),
            device: device(alias, true),
            display: display(on),
        }
    }

    fn evaluation(aliases: &[&str]) -> Evaluation {
        Evaluation::new(aliases.iter().map(|a| candidate(a, "A")).collect())
    }

    fn aliases(result: Option<MatchResult<'_>>) -> Vec<String> {
        match result {
            Some(MatchResult::One(c)) => vec![c.alias.clone()],
            Some(MatchResult::Many(cs)) => cs.iter().map(|c| c.alias.clone()).collect(),
            other => panic!("not a match: {other:?}"),
        }
    }

    #[test]
    fn ac9_one_match_selects_the_candidate() {
        let mut e = evaluation(&["lg", "dell"]);
        e.record(0, Answer::Matched(false));
        assert!(e.is_pending());
        assert_eq!(e.result(), None);
        e.record(1, Answer::Matched(true));
        assert_eq!(aliases(e.result()), ["dell"]);
        assert!(e.found("dell"));
        assert!(!e.found("lg"));
    }

    #[test]
    fn ac9_no_match_and_no_candidate_select_nothing() {
        let mut e = evaluation(&["lg"]);
        e.record(0, Answer::Matched(false));
        assert_eq!(e.result(), Some(MatchResult::NoMatch));
        assert_eq!(evaluation(&[]).result(), Some(MatchResult::NoMatch));
    }

    #[test]
    fn ac9_more_than_one_match_is_ambiguous_in_answer_sequence() {
        let mut e = evaluation(&["lg", "dell"]);
        e.record(1, Answer::Matched(true));
        e.record(0, Answer::Matched(true));
        assert!(matches!(e.result(), Some(MatchResult::Many(_))));
        assert_eq!(aliases(e.result()), ["dell", "lg"]);
        // ACT-6: an action that names one of the devices can start.
        assert!(e.found("dell"));
    }

    #[test]
    fn ac9_an_error_never_selects_and_the_last_error_stays() {
        let mut e = evaluation(&["lg", "dell", "tv"]);
        e.record(0, Answer::Failed("first".into()));
        // The error is visible before the evaluation finishes (UI-18).
        assert_eq!(e.error(), Some("lg: first"));
        assert_eq!(e.result(), None);
        e.record(1, Answer::Matched(true));
        e.record(2, Answer::Failed("second".into()));
        assert_eq!(e.result(), Some(MatchResult::Failed("tv: second")));
        assert!(!e.found("dell"));
    }

    #[test]
    fn a_second_answer_of_a_candidate_does_not_apply() {
        let mut e = evaluation(&["lg", "dell"]);
        e.record(0, Answer::Matched(true));
        e.record(0, Answer::Matched(true));
        assert!(e.is_pending());
        e.record(5, Answer::Matched(true));
        assert!(e.is_pending());
    }

    #[test]
    fn answer_from_output_keeps_the_error_text() {
        assert_eq!(
            Answer::from_output(&JobOutput::Operation(Ok(Outcome::Matched(true)))),
            Answer::Matched(true)
        );
        assert_eq!(
            Answer::from_output(&JobOutput::Operation(Err("timeout".into()))),
            Answer::Failed("timeout".into())
        );
        let Answer::Failed(text) = Answer::from_output(&JobOutput::Operation(Ok(Outcome::Done)))
        else {
            panic!("not a failure");
        };
        assert!(text.starts_with("unexpected "), "{text}");
    }

    #[test]
    fn flow2_start_makes_one_job_for_each_candidate_with_portal_match() {
        let mut m = PortalMatches::default();
        let owner = MatchOwner::Display(DisplayToken::from("A"));
        let mut none = candidate("tv", "A");
        none.device = device("tv", false);
        let jobs = m.start(
            owner.clone(),
            "menu-1",
            vec![candidate("lg", "A"), none, candidate("dell", "A")],
            &ComputerId::from("mac"),
            &[display("A")],
        );
        let aliases: Vec<_> = jobs
            .iter()
            .map(|(_, job)| {
                let Job::Operation { request, kind, .. } = job else {
                    panic!("not an operation");
                };
                assert_eq!(*kind, OperationKind::PortalMatch);
                let portal = request.portal.as_ref().unwrap();
                assert_eq!(portal.id, "menu-1");
                assert_eq!(portal.display_token, Some(DisplayToken::from("A")));
                request.device.alias.clone()
            })
            .collect();
        assert_eq!(aliases, ["lg", "dell"]);
        let mut tickets = jobs.into_iter().map(|(t, _)| t);
        let matched = JobOutput::Operation(Ok(Outcome::Matched(true)));
        assert_eq!(
            m.record(tickets.next().unwrap(), &matched),
            Some(owner.clone())
        );
        assert!(m.get(&owner).unwrap().is_pending());
        let no = JobOutput::Operation(Ok(Outcome::Matched(false)));
        m.record(tickets.next().unwrap(), &no);
        assert_eq!(aliases_of(&m, &owner), ["lg"]);
    }

    fn aliases_of(m: &PortalMatches, owner: &MatchOwner) -> Vec<String> {
        aliases(m.get(owner).unwrap().result())
    }

    #[test]
    fn flow4_a_result_of_an_older_evaluation_does_not_apply() {
        let mut m = PortalMatches::default();
        let owner = MatchOwner::Portal(PortalId(1));
        let local = ComputerId::from("mac");
        let old = m.start(
            owner.clone(),
            "portal-1",
            vec![candidate("lg", "A")],
            &local,
            &[],
        );
        let new = m.start(
            owner.clone(),
            "portal-2",
            vec![candidate("lg", "B")],
            &local,
            &[],
        );
        let matched = JobOutput::Operation(Ok(Outcome::Matched(true)));
        let (old, _) = old.into_iter().next().unwrap();
        assert_eq!(m.record(old, &matched), None);
        assert!(m.get(&owner).unwrap().is_pending());
        let (new, _) = new.into_iter().next().unwrap();
        assert_eq!(m.record(new.clone(), &matched), Some(owner.clone()));
        assert_eq!(aliases_of(&m, &owner), ["lg"]);
        // A removed evaluation takes no result.
        m.remove(&owner);
        assert_eq!(m.record(new, &matched), None);
    }

    #[test]
    fn ac9_a_second_answer_of_a_candidate_does_not_apply() {
        let mut evaluation = evaluation(&["One", "Two"]);
        evaluation.record(0, Answer::Matched(false));
        evaluation.record(0, Answer::Failed("late".into()));
        assert!(evaluation.is_pending(), "Two has no answer");
        assert_eq!(evaluation.error(), None);
        evaluation.record(1, Answer::Matched(true));
        assert!(!evaluation.is_pending());
        assert_eq!(
            evaluation
                .matched()
                .map(|c| c.alias.as_str())
                .collect::<Vec<_>>(),
            ["Two"]
        );
    }
}
