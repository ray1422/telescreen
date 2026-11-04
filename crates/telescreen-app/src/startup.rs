//! What the app does at the start, from the config file and the local computer ID
//! (PEER-3, PEER-5, UI-8). This module opens no window.

use std::path::Path;

use telescreen_core::config::Config;
use telescreen_core::ids::ComputerId;

/// The location of the example config, for a user with no config.
const EXAMPLE: &str = "https://github.com/ray1422/telescreen/blob/main/examples/config.yaml";

#[derive(Debug)]
pub enum Startup {
    /// The config and the local computer ID are known.
    Run {
        config: Config,
        computer: ComputerId,
    },
    /// The config is good, and the user must select the local computer ID.
    /// `notice` tells why, if the user selected one before.
    Choose {
        config: Config,
        notice: Option<String>,
    },
    /// The app cannot continue until the user corrects this.
    Problem(String),
}

/// `requested` is the `--computer` argument. `saved` is the saved computer ID.
/// `requested` has priority. Telescreen never takes the ID from the host name.
pub fn resolve(path: &Path, requested: Option<&str>, saved: Option<&str>) -> Startup {
    if !path.exists() {
        return Startup::Problem(format!(
            "There is no config at {}. Write it, then check again. An example: {EXAMPLE}",
            path.display()
        ));
    }
    let config = match Config::load(path) {
        Ok(config) => config,
        Err(e) => return Startup::Problem(e.to_string()),
    };
    if let Some(id) = requested {
        let computer = ComputerId::from(id);
        return if config.computer(&computer).is_some() {
            Startup::Run { config, computer }
        } else {
            Startup::Problem(format!("the config has no computer {id}"))
        };
    }
    let Some(id) = saved else {
        return Startup::Choose {
            config,
            notice: None,
        };
    };
    let computer = ComputerId::from(id);
    if config.computer(&computer).is_some() {
        Startup::Run { config, computer }
    } else {
        Startup::Choose {
            config,
            notice: Some(format!(
                "The config no longer has the computer {id}. Select this computer again."
            )),
        }
    }
}
