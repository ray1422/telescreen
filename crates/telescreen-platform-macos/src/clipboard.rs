//! The general macOS text clipboard (CLIP-7, CLIP-8, CLIP-11).

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;
use telescreen_core::clipboard::{ClipboardChange, ClipboardItem};
use telescreen_core::platform::{Clipboard, PlatformError};

/// The concealed mark of password managers (nspasteboard.org).
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

/// Polls the general pasteboard on the main thread (CLIP-8, IN-2).
#[derive(Debug, Default)]
pub struct MacClipboard {
    /// The change count of the last reported content or of the baseline.
    last_change: Option<isize>,
    /// The change count right after the last `set_text` (CLIP-11).
    own_change: Option<isize>,
}

impl MacClipboard {
    /// Starts closed (CLIP-1).
    pub fn new() -> Self {
        Self::default()
    }
}

fn pasteboard() -> Result<Retained<NSPasteboard>, PlatformError> {
    MainThreadMarker::new()
        .ok_or_else(|| PlatformError::Failed("the clipboard needs the main thread".into()))?;
    Ok(NSPasteboard::generalPasteboard())
}

impl Clipboard for MacClipboard {
    fn open(&mut self) -> Result<(), PlatformError> {
        // The current content is the baseline (CLIP-3).
        self.last_change = Some(pasteboard()?.changeCount());
        self.own_change = None;
        Ok(())
    }

    fn close(&mut self) {
        self.last_change = None;
        self.own_change = None;
    }

    fn changed(&mut self) -> Option<ClipboardChange> {
        let pasteboard = pasteboard().ok()?;
        let count = pasteboard.changeCount();
        if self.last_change == Some(count) {
            return None;
        }
        self.last_change = Some(count);
        let concealed_type = NSString::from_str(CONCEALED_TYPE);
        let concealed = pasteboard
            .types()
            .is_some_and(|types| types.containsObject(&concealed_type));
        // SAFETY: AppKit provides this immutable string constant for the process lifetime.
        let Some(text) = pasteboard.stringForType(unsafe { NSPasteboardTypeString }) else {
            return Some(ClipboardChange::Cleared);
        };
        Some(ClipboardChange::Text(ClipboardItem {
            text: text.to_string(),
            concealed,
        }))
    }

    fn set_text(&mut self, text: &str, concealed: bool) -> Result<(), PlatformError> {
        let pasteboard = pasteboard()?;
        let text = NSString::from_str(text);
        pasteboard.clearContents();
        // SAFETY: AppKit provides this immutable string constant for the process lifetime.
        let mut written = pasteboard.setString_forType(&text, unsafe { NSPasteboardTypeString });
        if concealed {
            // The convention marks the item with the type; the data is not used.
            written &=
                pasteboard.setString_forType(&NSString::new(), &NSString::from_str(CONCEALED_TYPE));
        }
        if !written {
            return Err(PlatformError::Failed(
                "cannot write the clipboard text".into(),
            ));
        }
        self.own_change = Some(pasteboard.changeCount());
        Ok(())
    }

    fn clear_if_owned(&mut self) {
        let Ok(pasteboard) = pasteboard() else {
            return;
        };
        // A different count means that some other program wrote the clipboard.
        if self.own_change.take() == Some(pasteboard.changeCount()) {
            pasteboard.clearContents();
        }
    }
}
