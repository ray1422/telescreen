//! The invisible keyboard focus holder on the target (FLOW-23, UI-7).

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSScreen, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use telescreen_core::platform::{DisplayInfo, FocusHolder, PlatformError};

define_class!(
    // SAFETY: this main-thread panel has no ivars or custom destruction.
    #[unsafe(super = NSPanel)]
    #[name = "TelescreenFocusPanel"]
    #[derive(Debug)]
    struct FocusPanel;

    impl FocusPanel {
        // SAFETY: the signature matches NSWindow's AppKit method.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        // SAFETY: the signature matches NSWindow's AppKit method.
        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }
);

/// Holds keyboard focus without activating the app (FLOW-23, UI-7).
#[derive(Debug, Default)]
pub struct MacFocusHolder {
    panel: Option<Retained<FocusPanel>>,
    held: bool,
}

impl MacFocusHolder {
    /// Starts without a panel or a hold (UI-7).
    pub fn new() -> Self {
        Self::default()
    }
}

impl FocusHolder for MacFocusHolder {
    fn hold(&mut self, display: &DisplayInfo) -> Result<(), PlatformError> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            PlatformError::Failed("the focus holder needs the main thread".into())
        })?;
        let bounds = display.bounds.ok_or_else(|| {
            PlatformError::Failed("the focus holder display has no coordinates".into())
        })?;
        let primary = NSScreen::screens(mtm).firstObject().ok_or_else(|| {
            PlatformError::Failed("the focus holder cannot find the primary display".into())
        })?;
        let frame = primary.frame();
        // CoreGraphics has y down from the primary display's top (IN-11).
        let top_left = NSPoint::new(bounds.x, frame.origin.y + frame.size.height - bounds.y);
        let panel = match self.panel {
            Some(ref panel) => panel,
            None => self.panel.insert(make_panel(mtm)?),
        };
        panel.setFrameTopLeftPoint(top_left);
        panel.orderFrontRegardless();
        panel.makeKeyWindow();
        self.held = true;
        Ok(())
    }

    fn release(&mut self) {
        if !self.held || MainThreadMarker::new().is_none() {
            return;
        }
        if let Some(panel) = &self.panel {
            panel.orderOut(None);
        }
        self.held = false;
    }
}

/// Makes a transparent panel that can take keyboard focus (UI-7).
fn make_panel(mtm: MainThreadMarker) -> Result<Retained<FocusPanel>, PlatformError> {
    let allocated = FocusPanel::alloc(mtm).set_ivars(());
    // SAFETY: this calls the NSWindow initializer on a new NSPanel subclass.
    // The borderless panel has a valid one-point content rectangle.
    let panel: Option<Retained<FocusPanel>> = unsafe {
        msg_send![super(allocated),
            initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)),
            styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };
    let panel =
        panel.ok_or_else(|| PlatformError::Failed("cannot make the focus holder panel".into()))?;
    // SAFETY: Retained owns the panel until the focus holder is dropped.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    Ok(panel)
}
