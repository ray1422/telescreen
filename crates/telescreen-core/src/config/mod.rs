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

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use crate::ids::{ComputerId, DeviceId};
use crate::keys::SpecialKeyGroup;

/// The only config format version that this build accepts.
pub const CONFIG_VERSION: u64 = 1;

/// The default operation timeout (`docs/config.md`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

/// The default timeout of `verify`: a verifier can wait for the user (`docs/config.md`).
pub const VERIFY_TIMEOUT: Duration = Duration::from_millis(60_000);

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

/// A platform workaround. Each one is on, unless `disabled-quirks` has it (IN-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Quirk {
    /// A scroll on KWin as a button scroll, not as a wheel scroll (IN-15).
    KwinButtonScroll,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Computer {
    pub label: Option<String>,
    /// Plain text for the `prompt` verifier of this computer (VER-5).
    pub hint: Option<String>,
    /// The directories to find a program name with no `/`. `None` means the
    /// `PATH` of the login shell (`docs/config.md`, "Programs and the search path").
    pub path: Option<Vec<PathBuf>>,
    /// Clipboard sync of this computer (CLIP-1).
    #[serde(default)]
    pub clipboard: ClipboardConfig,
    /// The multiplier for each scroll that this computer receives (IN-14).
    #[serde(default = "one")]
    pub scroll_speed: f32,
    /// The platform workarounds that this computer does not use (IN-16).
    #[serde(default)]
    pub disabled_quirks: BTreeSet<Quirk>,
    /// Where each special key goes while this computer is the source (IN-20).
    #[serde(default)]
    pub special_keys: SpecialKeys,
    pub endpoint: Endpoint,
    /// Keys are local aliases. They have no OS meaning.
    /// In the order of the config file (UI-18: the menu lists the displays
    /// in this order).
    pub devices: IndexMap<String, Device>,
}

/// The `clipboard` key of a computer (CLIP-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipboardConfig {
    /// The start value of the text clipboard switch. The default is on.
    #[serde(default = "on")]
    pub text: bool,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self { text: on() }
    }
}

/// The `special-keys` key of a computer (IN-20). It applies only when this
/// computer is the source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SpecialKeys {
    /// The groups that go to the target and not to this computer.
    #[serde(default)]
    pub send_to_target: BTreeSet<SpecialKeyGroup>,
    /// The groups that go to this computer and not to the target.
    #[serde(default)]
    pub keep_on_source: BTreeSet<SpecialKeyGroup>,
}

/// What the source does with a special key while a forwarding portal has the
/// keyboard focus (IN-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKeyAction {
    /// Forward the key, and suppress it on the source.
    SendToTarget,
    /// Let the key go to the source, and do not forward it.
    KeepOnSource,
    /// Suppress the key, and do not forward it.
    Suppress,
}

impl SpecialKeys {
    /// The action for a group.
    pub fn action(&self, group: SpecialKeyGroup) -> SpecialKeyAction {
        if self.send_to_target.contains(&group) {
            SpecialKeyAction::SendToTarget
        } else if self.keep_on_source.contains(&group) {
            SpecialKeyAction::KeepOnSource
        } else {
            SpecialKeyAction::Suppress
        }
    }
}

fn on() -> bool {
    true
}

impl Computer {
    /// Whether this computer uses a platform workaround (IN-16).
    pub fn uses_quirk(&self, quirk: Quirk) -> bool {
        !self.disabled_quirks.contains(&quirk)
    }
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    /// The `host` key: one host or a list (NET-11). It is empty only with `discovery`.
    #[serde(rename = "host", default, deserialize_with = "one_or_many")]
    pub hosts: Vec<String>,
    pub port: u16,
    /// Other computers can find this computer (NET-13).
    #[serde(default)]
    pub discovery: Option<Discovery>,
}

/// The `endpoint.discovery` key (NET-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Discovery {
    Mdns,
}

fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Hosts {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Hosts::deserialize(deserializer)? {
        Hosts::One(host) => vec![host],
        Hosts::Many(hosts) => hosts,
    })
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Device {
    pub id: DeviceId,
    pub label: Option<String>,
    pub portal_match: Option<Operation>,
    pub input_target: Option<Operation>,
    /// This computer runs it when the device switches to this computer (VER-1).
    #[serde(default, deserialize_with = "verify_operation")]
    pub verify: Option<Operation>,
    #[serde(default)]
    pub handoff: BTreeMap<ComputerId, Handoff>,
}

/// The route of this computer to switch a device to a different computer. It has
/// no `verify`: the destination verifies (ADR 0009).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    pub steps: Vec<Operation>,
}

/// An abstract operation that a driver implements.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "RawOperation")]
pub struct Operation {
    pub driver: Driver,
    pub timeout: Duration,
}

/// A driver defines how Telescreen reads the result of a program (`docs/config.md`).
#[derive(Debug, Clone, PartialEq)]
pub enum Driver {
    /// Only the exit code: 0 is success or true.
    Exec(Program),
    /// Protocol 2: JSON on stdin and stdout.
    Command(Program),
    /// The built-in verifier (VER-5). Only in `verify`.
    Prompt,
    /// The built-in display matcher. Only in `portal-match` and
    /// `input-target`. A display matches when its metadata has each value of
    /// `matches`, with the same JSON type.
    Display { matches: Map<String, Value> },
    /// A namespaced extension driver, for example `lab.http`. This build has no
    /// extension drivers; the capability check reports them as unsupported.
    Extension { name: String, options: Value },
}

/// The program of an `exec` or `command` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The program and its arguments. A `command` string is already split.
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

/// `command` is a string or a list, as in Docker Compose.
#[derive(Deserialize)]
#[serde(untagged)]
enum CommandLine {
    Text(String),
    List(Vec<String>),
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RawOperation {
    driver: String,
    timeout_ms: Option<u64>,
    command: Option<CommandLine>,
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    options: Option<Value>,
    #[serde(rename = "match")]
    matches: Option<Map<String, Value>>,
}

impl RawOperation {
    fn into_operation(self, default_timeout: Duration) -> Result<Operation, String> {
        let program = |command: Option<CommandLine>| -> Result<Program, String> {
            let argv = match command.ok_or("the driver needs `command`")? {
                // POSIX shell rules, but no shell runs it.
                CommandLine::Text(text) => {
                    shlex::split(&text).ok_or("cannot split the command string")?
                }
                CommandLine::List(list) => list,
            };
            if argv.is_empty() {
                return Err("the command is empty".into());
            }
            Ok(Program {
                argv,
                cwd: self.cwd.clone(),
                env: self.env.clone(),
            })
        };
        let driver = match self.driver.as_str() {
            "exec" => Driver::Exec(program(self.command)?),
            "command" => Driver::Command(program(self.command)?),
            "prompt" => Driver::Prompt,
            "display" => Driver::Display {
                matches: self
                    .matches
                    .filter(|matches| !matches.is_empty())
                    .ok_or("the display driver needs a `match` with one key or more")?,
            },
            _ => Driver::Extension {
                options: self.options.unwrap_or(Value::Null),
                name: self.driver,
            },
        };
        let timeout = self
            .timeout_ms
            .map_or(default_timeout, Duration::from_millis);
        Ok(Operation { driver, timeout })
    }
}

impl TryFrom<RawOperation> for Operation {
    type Error = String;

    fn try_from(raw: RawOperation) -> Result<Self, Self::Error> {
        raw.into_operation(DEFAULT_TIMEOUT)
    }
}

fn verify_operation<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Operation>, D::Error> {
    Option::<RawOperation>::deserialize(d)?
        .map(|raw| raw.into_operation(VERIFY_TIMEOUT))
        .transpose()
        .map_err(serde::de::Error::custom)
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
