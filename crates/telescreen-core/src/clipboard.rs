//! Text clipboard sync rules of one peer (CLIP-1 to CLIP-7).

/// The largest text in bytes of UTF-8 (CLIP-6).
pub const MAX_TEXT: usize = 256 * 1024;

/// A local text clipboard item (CLIP-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardItem {
    pub text: String,
    pub concealed: bool,
}

/// The result of a local clipboard change (CLIP-2, CLIP-6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    Send(String),
    TooLarge { bytes: usize },
}

/// Tracks the switch and the next applied text to suppress (CLIP-2, CLIP-4).
#[derive(Debug)]
pub struct ClipboardSync {
    enabled: bool,
    applied: Option<String>,
}

impl ClipboardSync {
    /// Starts with the switch from the local config (CLIP-1).
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            applied: None,
        }
    }

    /// Returns the current switch value (CLIP-1).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Changes the switch (CLIP-1).
    /// Turning it off forgets the applied text (CLIP-5).
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.applied = None;
        }
    }

    /// Filters a local change for sending (CLIP-2, CLIP-4, CLIP-6, CLIP-7).
    pub fn local_change(&mut self, item: &ClipboardItem) -> Option<Outgoing> {
        let applied = self.applied.take();
        if !self.enabled
            || item.concealed
            || item.text.is_empty()
            || applied.as_ref() == Some(&item.text)
        {
            return None;
        }
        if item.text.len() > MAX_TEXT {
            return Some(Outgoing::TooLarge {
                bytes: item.text.len(),
            });
        }
        Some(Outgoing::Send(item.text.clone()))
    }

    /// Accepts a received text and remembers its next local change (CLIP-2, CLIP-4).
    pub fn received(&mut self, text: &str) -> bool {
        if !self.enabled || text.len() > MAX_TEXT {
            return false;
        }
        self.applied = Some(text.to_owned());
        true
    }
}
