//! The text clipboard: open, sync, and receive (spec section 4.4).

use telescreen_core::clipboard::{ClipboardChange, Outgoing};
use telescreen_core::ids::{ComputerId, SessionId};
use telescreen_core::session::source::SourcePhase;
use telescreen_core::session::target::TargetPhase;
use telescreen_core::wire::Message;
use telescreen_transport::NetCommand;

use super::Controller;

impl Controller {
    /// Opens the clipboard only while sync is possible. A failed open is retried
    /// only after sync becomes unavailable and available again (CLIP-3, CLIP-5).
    pub(super) fn update_clipboard(&mut self) {
        let sessions = self.clipboard_sessions();
        let can_sync =
            self.clipboard.enabled() && self.ports.clipboard.is_some() && !sessions.is_empty();
        if !can_sync {
            if self.clipboard_open {
                if let Some(clipboard) = self.ports.clipboard.as_mut() {
                    clipboard.clear_if_owned();
                    clipboard.close();
                }
                self.clipboard_open = false;
                self.clipboard.reset();
            }
            self.clipboard_open_failed = false;
            self.clipboard_session = None;
            return;
        }
        if !self.clipboard_open
            && !self.clipboard_open_failed
            && let Some(clipboard) = self.ports.clipboard.as_mut()
        {
            match clipboard.open() {
                Ok(()) => self.clipboard_open = true,
                Err(error) => {
                    self.clipboard_open_failed = true;
                    self.notice(format!("Cannot open the clipboard: {error}"));
                }
            }
        }
        if self.clipboard_open
            && self
                .clipboard_session
                .is_some_and(|session| !sessions.iter().any(|(id, _)| *id == session))
        {
            if let Some(clipboard) = self.ports.clipboard.as_mut() {
                clipboard.clear_if_owned();
            }
            self.clipboard_session = None;
        }
    }

    /// Lists the sessions that can sync text (CLIP-2).
    fn clipboard_sessions(&self) -> Vec<(SessionId, ComputerId)> {
        let mut sessions: Vec<_> = self
            .portals
            .values()
            .filter_map(|p| p.session.as_ref())
            .filter(|source| source.phase() == SourcePhase::Forwarding)
            .filter_map(|source| Some((source.session()?, source.target().clone())))
            .collect();
        sessions.extend(
            self.targets()
                .filter(|(_, target)| target.phase() == TargetPhase::Forwarding)
                .map(|(session, target)| (session, target.source().clone())),
        );
        sessions
    }

    /// Polls once and sends a local change to forwarding sessions (CLIP-2, CLIP-5).
    /// The app reads the clipboard only with the switch on and a session that can
    /// sync (CLIP-1, CLIP-5).
    pub(super) fn sync_clipboard(&mut self) {
        if !self.clipboard_open {
            return;
        }
        let sessions = self.clipboard_sessions();
        let Some(item) = self.ports.clipboard.as_mut().and_then(|c| c.changed()) else {
            return;
        };
        let outgoing = self.clipboard.local_change(&item);
        // An empty result for nonempty text is the core suppressing our write.
        // Clears and empty text also replace the last applied session's item.
        if !outgoing.is_empty()
            || !matches!(&item, ClipboardChange::Text(item) if !item.text.is_empty())
        {
            self.clipboard_session = None;
        }
        for outgoing in outgoing {
            match outgoing {
                Outgoing::Send { text, concealed } => {
                    for (session, peer) in &sessions {
                        self.ports.net.send(NetCommand::Send {
                            peer: peer.clone(),
                            message: Message::ClipboardText {
                                session: *session,
                                text: text.clone(),
                                concealed,
                            },
                        });
                    }
                }
                Outgoing::Clear => {
                    for (session, peer) in &sessions {
                        self.ports.net.send(NetCommand::Send {
                            peer: peer.clone(),
                            message: Message::ClipboardClear { session: *session },
                        });
                    }
                }
                Outgoing::TooLarge { bytes } => {
                    self.notice(format!(
                        "Cannot sync clipboard text of {bytes} bytes: it is too large."
                    ));
                }
            }
        }
    }

    /// Applies text only from the peer of a forwarding session (CLIP-2, CLIP-5).
    pub(super) fn receive_clipboard(
        &mut self,
        peer: &ComputerId,
        session: SessionId,
        text: &str,
        concealed: bool,
    ) {
        self.update_clipboard();
        if !self.clipboard_open
            || !self
                .clipboard_sessions()
                .iter()
                .any(|(id, computer)| *id == session && computer == peer)
        {
            return;
        }
        if let Some(clipboard) = self.ports.clipboard.as_mut()
            && self.clipboard.received(text)
        {
            match clipboard.set_text(text, concealed) {
                Ok(()) => self.clipboard_session = Some(session),
                // The error names no clipboard text (CLIP-9).
                Err(error) => self.notice(format!("Cannot set clipboard text: {error}")),
            }
        }
    }

    /// Clears only for the authenticated peer of a forwarding session (CLIP-7).
    pub(super) fn receive_clipboard_clear(&mut self, peer: &ComputerId, session: SessionId) {
        self.update_clipboard();
        if self.clipboard_open
            && self
                .clipboard_sessions()
                .iter()
                .any(|(id, computer)| *id == session && computer == peer)
            && self.clipboard.received_clear()
            && let Some(clipboard) = self.ports.clipboard.as_mut()
        {
            clipboard.clear_if_owned();
            self.clipboard_session = None;
        }
    }
}
