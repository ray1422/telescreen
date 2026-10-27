//! The general macOS text clipboard (CLIP-7, CLIP-8).

use objc2::MainThreadMarker;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;
use telescreen_core::clipboard::ClipboardItem;
use telescreen_core::platform::{Clipboard, PlatformError};

/// Polls the general pasteboard on the main thread (CLIP-8, IN-2).
#[derive(Debug, Default)]
pub struct MacClipboard {
    last_change: Option<isize>,
}

impl MacClipboard {
    /// Starts without a known pasteboard change count (CLIP-8).
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clipboard for MacClipboard {
    fn changed(&mut self) -> Option<ClipboardItem> {
        let _main_thread = MainThreadMarker::new()?;
        let pasteboard = NSPasteboard::generalPasteboard();
        let count = pasteboard.changeCount();
        if self.last_change == Some(count) {
            return None;
        }
        self.last_change = Some(count);
        let concealed_type = NSString::from_str("org.nspasteboard.ConcealedType");
        let concealed = pasteboard
            .types()
            .is_some_and(|types| types.containsObject(&concealed_type));
        // SAFETY: AppKit provides this immutable string constant for the process lifetime.
        let text = pasteboard.stringForType(unsafe { NSPasteboardTypeString })?;
        Some(ClipboardItem {
            text: text.to_string(),
            concealed,
        })
    }

    fn set_text(&mut self, text: &str) -> Result<(), PlatformError> {
        let _main_thread = MainThreadMarker::new()
            .ok_or_else(|| PlatformError::Failed("the clipboard needs the main thread".into()))?;
        let pasteboard = NSPasteboard::generalPasteboard();
        let text = NSString::from_str(text);
        pasteboard.clearContents();
        // SAFETY: AppKit provides this immutable string constant for the process lifetime.
        if pasteboard.setString_forType(&text, unsafe { NSPasteboardTypeString }) {
            Ok(())
        } else {
            Err(PlatformError::Failed(
                "cannot write the clipboard text".into(),
            ))
        }
    }
}
