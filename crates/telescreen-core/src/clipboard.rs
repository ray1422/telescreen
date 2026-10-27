//! Text clipboard sync rules of one peer (CLIP-1 to CLIP-7).

/// The largest text in bytes of UTF-8 (CLIP-6).
pub const MAX_TEXT: usize = 256 * 1024;

/// A local text clipboard item (CLIP-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardItem {
    pub text: String,
    pub concealed: bool,
}

/// A change of the local clipboard (CLIP-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardChange {
    /// The clipboard holds text.
    Text(ClipboardItem),
    /// The clipboard holds no text: it was cleared, or it holds a different type.
    Cleared,
}

/// The result of a local clipboard change (CLIP-2, CLIP-6, CLIP-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    Send {
        text: String,
        concealed: bool,
    },
    /// The concealed text that this peer sent went off the local clipboard (CLIP-7).
    Clear,
    TooLarge {
        bytes: usize,
    },
}

/// Tracks the switch, the next applied text to suppress, and the sent concealed
/// text (CLIP-2, CLIP-4, CLIP-7).
#[derive(Debug)]
pub struct ClipboardSync {
    enabled: bool,
    applied: Option<String>,
    concealed_sent: bool,
}

impl ClipboardSync {
    /// Starts with the switch from the local config (CLIP-1).
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            applied: None,
            concealed_sent: false,
        }
    }

    /// Returns the current switch value (CLIP-1).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Changes the switch (CLIP-1).
    /// Turning it off forgets the applied and the sent text (CLIP-5).
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.reset();
        }
    }

    /// Forgets the applied and the sent text, for example when the sync stops
    /// (CLIP-5, CLIP-11).
    pub fn reset(&mut self) {
        self.applied = None;
        self.concealed_sent = false;
    }

    /// Filters a local change for sending (CLIP-2, CLIP-4, CLIP-6, CLIP-7).
    pub fn local_change(&mut self, change: &ClipboardChange) -> Vec<Outgoing> {
        let applied = self.applied.take();
        if !self.enabled {
            return Vec::new();
        }
        let mut out = Vec::new();
        let item = match change {
            ClipboardChange::Text(item) if !item.text.is_empty() => item,
            _ => {
                if std::mem::take(&mut self.concealed_sent) {
                    out.push(Outgoing::Clear);
                }
                return out;
            }
        };
        if applied.as_ref() == Some(&item.text) {
            return out;
        }
        if item.text.len() > MAX_TEXT {
            // The other peer keeps no concealed text that is gone here (CLIP-7).
            if std::mem::take(&mut self.concealed_sent) {
                out.push(Outgoing::Clear);
            }
            out.push(Outgoing::TooLarge {
                bytes: item.text.len(),
            });
            return out;
        }
        // A new text replaces the old one on the other peer.
        self.concealed_sent = item.concealed;
        out.push(Outgoing::Send {
            text: item.text.clone(),
            concealed: item.concealed,
        });
        out
    }

    /// Accepts a received text and remembers its next local change (CLIP-2, CLIP-4).
    /// The received text replaces a concealed text that this peer sent, on both peers.
    pub fn received(&mut self, text: &str) -> bool {
        if !self.enabled || text.len() > MAX_TEXT {
            return false;
        }
        self.applied = Some(text.to_owned());
        self.concealed_sent = false;
        true
    }

    /// Accepts a received clear (CLIP-7).
    pub fn received_clear(&self) -> bool {
        self.enabled
    }
}
