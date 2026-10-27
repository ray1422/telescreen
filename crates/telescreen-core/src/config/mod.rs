//! The config contract (`docs/config.md`, `schema/config.schema.json`).
//!
//! Validation has layers (`docs/config.md`, "Validation Layers"):
//!
//! 1. [`yaml`]: strict YAML to JSON-compatible data.
//! 2. [`schema`]: JSON Schema (the same schema file as the Python tools).
//! 3. Typed model: [`Config`] and its parts (serde).
//! 4. [`semantic`]: cross-reference checks.
//!
//! The capability check (layer 4 in `docs/config.md`) is not in this module.

pub mod schema;
pub mod semantic;
pub mod yaml;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{ComputerId, DeviceId};

/// The only config format version that this build accepts.
pub const CONFIG_VERSION: u64 = 1;

/// The default operation timeout (`docs/config.md`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

/// A config error. Each variant is one validation layer.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("YAML: {0}")]
    Yaml(String),
    #[error("schema: {}", .0.join("; "))]
    Schema(Vec<String>),
    #[error("model: {0}")]
    Model(String),
    #[error("semantic: {}", .0.join("; "))]
    Semantic(Vec<String>),
}

/// A validated config.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u64,
    pub computers: BTreeMap<ComputerId, Computer>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Computer {
    pub label: Option<String>,
    /// Plain text for the confirmation prompt of this computer (FLOW-7).
    pub hint: Option<String>,
    /// Clipboard sync of this computer (CLIP-1).
    #[serde(default)]
    pub clipboard: ClipboardConfig,
    pub endpoint: Endpoint,
    /// Keys are local aliases. They have no OS meaning.
    pub devices: BTreeMap<String, Device>,
}

/// The `clipboard` key of a computer (CLIP-1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipboardConfig {
    /// The start value of the text clipboard switch. The default is off.
    #[serde(default)]
    pub text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Device {
    pub id: DeviceId,
    pub label: Option<String>,
    pub portal_match: Option<Operation>,
    pub input_target: Option<Operation>,
    #[serde(default)]
    pub handoff: BTreeMap<ComputerId, Handoff>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    pub steps: Vec<Operation>,
    pub verify: Option<Operation>,
}

/// An abstract operation that a driver implements.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "RawOperation")]
pub struct Operation {
    pub driver: Driver,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Driver {
    /// The baseline `command` driver.
    Command(CommandOptions),
    /// A namespaced extension driver, for example `lab.http`. This build has no
    /// extension drivers; the capability check reports them as unsupported.
    Extension { name: String, options: Value },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOptions {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RawOperation {
    driver: String,
    timeout_ms: Option<u64>,
    options: Value,
}

impl TryFrom<RawOperation> for Operation {
    type Error = String;

    fn try_from(raw: RawOperation) -> Result<Self, Self::Error> {
        let driver = if raw.driver == "command" {
            Driver::Command(serde_json::from_value(raw.options).map_err(|e| e.to_string())?)
        } else {
            Driver::Extension {
                name: raw.driver,
                options: raw.options,
            }
        };
        let timeout = raw
            .timeout_ms
            .map_or(DEFAULT_TIMEOUT, Duration::from_millis);
        Ok(Self { driver, timeout })
    }
}

impl Config {
    /// Runs all layers on YAML text.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        Self::from_value(yaml::to_json(text)?)
    }

    /// Reads and parses a config file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&text)
    }

    /// Runs the schema, model, and semantic layers on parsed data.
    pub fn from_value(value: Value) -> Result<Self, ConfigError> {
        let errors = schema::errors(&value);
        if !errors.is_empty() {
            return Err(ConfigError::Schema(errors));
        }
        let config: Self =
            serde_json::from_value(value).map_err(|e| ConfigError::Model(e.to_string()))?;
        if config.version != CONFIG_VERSION {
            return Err(ConfigError::Schema(vec![format!(
                "unsupported version {}",
                config.version
            )]));
        }
        let errors = semantic::errors(&config);
        if !errors.is_empty() {
            return Err(ConfigError::Semantic(errors));
        }
        Ok(config)
    }

    pub fn computer(&self, id: &ComputerId) -> Option<&Computer> {
        self.computers.get(id)
    }
}

impl Computer {
    /// Finds the local device that has the shared device ID.
    pub fn device_by_id(&self, id: &DeviceId) -> Option<(&str, &Device)> {
        self.devices
            .iter()
            .find(|(_, d)| &d.id == id)
            .map(|(a, d)| (a.as_str(), d))
    }
}
