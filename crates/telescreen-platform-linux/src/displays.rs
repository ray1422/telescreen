//! The outputs as displays (`docs/config.md`, driver request `displays`).
//!
//! xdg-output gives the logical position and size: local desktop logical
//! coordinates. The token is the output name, for example `HDMI-A-2`. The
//! metadata has the name, so a helper can select the input-target by name.

use serde_json::{Map, Value};
use telescreen_core::ids::DisplayToken;
use telescreen_core::platform::{DisplayInfo, Displays, PlatformError};
use telescreen_core::pointer::Rect;

use crate::wl::{Globals, OutputData};

#[derive(Debug, Default)]
pub struct LinuxDisplays;

impl Displays for LinuxDisplays {
    fn displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        let globals = Globals::connect().map_err(PlatformError::Failed)?;
        Ok(globals
            .state
            .outputs
            .iter()
            .filter_map(|(_, data)| info(data))
            .collect())
    }
}

fn info(data: &OutputData) -> Option<DisplayInfo> {
    let name = data.name.clone()?;
    let bounds = match (data.logical_position, data.logical_size) {
        (Some((x, y)), Some((w, h))) => Some(Rect {
            x: f64::from(x),
            y: f64::from(y),
            width: f64::from(w),
            height: f64::from(h),
        }),
        _ => None,
    };
    let scale = match (data.mode, data.logical_size) {
        (Some((mode_w, _)), Some((logical_w, _))) if logical_w > 0 => {
            f64::from(mode_w) / f64::from(logical_w)
        }
        _ => 1.0,
    };
    let mut metadata = Map::new();
    metadata.insert("name".into(), Value::String(name.clone()));
    for (key, value) in [
        ("description", &data.description),
        ("make", &data.make),
        ("model", &data.model),
    ] {
        if let Some(value) = value {
            metadata.insert(key.into(), Value::String(value.clone()));
        }
    }
    Some(DisplayInfo {
        token: DisplayToken(name),
        bounds,
        scale,
        metadata,
    })
}
