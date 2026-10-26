//! The display list (`docs/config.md`, driver request `displays`).
//!
//! CoreGraphics global display coordinates have the origin at the top-left of the
//! main display and y down, so they are local desktop logical coordinates.

use serde_json::{Map, Value, json};
use telescreen_core::ids::DisplayToken;
use telescreen_core::platform::{DisplayInfo, Displays, PlatformError};
use telescreen_core::pointer::Rect;

use crate::ffi;

/// The token of a CoreGraphics display ID. It is valid only for this run.
pub fn token(id: ffi::CGDirectDisplayID) -> DisplayToken {
    DisplayToken(format!("cg-{id}"))
}

/// The CoreGraphics display ID of a token.
pub fn display_id(token: &DisplayToken) -> Option<ffi::CGDirectDisplayID> {
    token.0.strip_prefix("cg-")?.parse().ok()
}

#[derive(Debug, Default)]
pub struct MacDisplays;

impl Displays for MacDisplays {
    fn displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        let mut ids = [0u32; 32];
        let mut count = 0u32;
        // SAFETY: the buffer has room for 32 IDs, and count receives the number.
        let error = unsafe { ffi::CGGetActiveDisplayList(32, ids.as_mut_ptr(), &mut count) };
        if error != 0 {
            return Err(PlatformError::Failed(format!(
                "CGGetActiveDisplayList: {error}"
            )));
        }
        Ok(ids
            .iter()
            .take(count as usize)
            .map(|&id| info(id))
            .collect())
    }
}

fn info(id: ffi::CGDirectDisplayID) -> DisplayInfo {
    // SAFETY: read-only queries of an active display ID. The mode is released.
    let (bounds, scale, vendor, model, serial, builtin, rotation) = unsafe {
        let bounds = ffi::CGDisplayBounds(id);
        let mode = ffi::CGDisplayCopyDisplayMode(id);
        let scale = if mode.is_null() {
            1.0
        } else {
            let pixels = ffi::CGDisplayModeGetPixelWidth(mode) as f64;
            let points = ffi::CGDisplayModeGetWidth(mode) as f64;
            ffi::CGDisplayModeRelease(mode);
            if points > 0.0 { pixels / points } else { 1.0 }
        };
        (
            bounds,
            scale,
            ffi::CGDisplayVendorNumber(id),
            ffi::CGDisplayModelNumber(id),
            ffi::CGDisplaySerialNumber(id),
            ffi::CGDisplayIsBuiltin(id) != 0,
            ffi::CGDisplayRotation(id),
        )
    };
    let mut metadata = Map::new();
    metadata.insert("vendor".into(), json!(vendor));
    metadata.insert("model".into(), json!(model));
    metadata.insert("serial".into(), json!(serial));
    metadata.insert("builtin".into(), json!(builtin));
    metadata.insert("rotation".into(), json!(rotation));
    if let Some(name) = crate::window::screen_name(id) {
        metadata.insert("name".into(), Value::String(name));
    }
    DisplayInfo {
        token: token(id),
        bounds: Some(Rect {
            x: bounds.origin.x,
            y: bounds.origin.y,
            width: bounds.size.width,
            height: bounds.size.height,
        }),
        scale,
        metadata,
    }
}
