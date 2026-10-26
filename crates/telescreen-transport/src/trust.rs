//! The local trust list (ADR 0004, NET-3): the fingerprints that this peer accepts
//! for each computer ID. It is in the local data directory, not in the config.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use telescreen_core::ids::ComputerId;

use crate::Fingerprint;

#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Format { path: PathBuf, message: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustStore {
    version: u32,
    peers: BTreeMap<ComputerId, BTreeSet<Fingerprint>>,
}

impl TrustStore {
    /// Reads the trust list. A missing file is an empty list.
    pub fn load(path: &Path) -> Result<Self, TrustError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty()),
            Err(source) => {
                return Err(TrustError::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let store: Self = serde_json::from_str(&text).map_err(|e| TrustError::Format {
            path: path.to_owned(),
            message: e.to_string(),
        })?;
        if store.version != 1 {
            return Err(TrustError::Format {
                path: path.to_owned(),
                message: format!("unknown version {}", store.version),
            });
        }
        Ok(store)
    }

    /// Writes the trust list through a temporary file, so that a crash does not
    /// leave a partial file.
    pub fn save(&self, path: &Path) -> Result<(), TrustError> {
        let io = |source| TrustError::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| TrustError::Format {
            path: path.to_owned(),
            message: e.to_string(),
        })?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text + "\n").map_err(io)?;
        fs::rename(&tmp, path).map_err(io)
    }

    pub fn empty() -> Self {
        Self {
            version: 1,
            peers: BTreeMap::new(),
        }
    }

    /// Adds a fingerprint for a computer. Returns false if it was already there.
    pub fn trust(&mut self, computer: ComputerId, fingerprint: Fingerprint) -> bool {
        self.peers.entry(computer).or_default().insert(fingerprint)
    }

    pub fn is_trusted(&self, computer: &ComputerId, fingerprint: &Fingerprint) -> bool {
        self.peers
            .get(computer)
            .is_some_and(|set| set.contains(fingerprint))
    }

    /// The computers that this fingerprint is trusted for.
    pub fn computers_for(&self, fingerprint: &Fingerprint) -> Vec<&ComputerId> {
        self.peers
            .iter()
            .filter(|(_, set)| set.contains(fingerprint))
            .map(|(c, _)| c)
            .collect()
    }

    pub fn peers(&self) -> impl Iterator<Item = (&ComputerId, &BTreeSet<Fingerprint>)> {
        self.peers.iter()
    }
}
