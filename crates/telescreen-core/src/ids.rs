//! Identifiers of the domain model. See `GLOSSARY.md`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A stable computer ID: a key of `computers` in the config.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComputerId(pub String);

/// A shared physical device ID: the `id` of a device in the config.
/// The same physical device has the same ID on each computer that connects to it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub String);

/// A session: the generation number that the target gave with the lease (NET-6).
/// A larger generation is a newer session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub u64);

/// An opaque display token from the current OS display enumeration of one computer.
/// It is not valid on a different computer or after a restart (`docs/config.md`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DisplayToken(pub String);

macro_rules! display_as_inner {
    ($($t:ty),*) => {$(
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    )*};
}
display_as_inner!(ComputerId, DeviceId, SessionId, DisplayToken);

impl From<&str> for ComputerId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<&str> for DeviceId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<&str> for DisplayToken {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}
