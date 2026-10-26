//! Certificate fingerprints (ADR 0004).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The SHA-256 of a DER certificate. The text form is `sha256:` and 64 hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of(der: &[u8]) -> Self {
        Self(Sha256::digest(der).into())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sha256:")?;
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a fingerprint is `sha256:` and 64 hex digits: {0}")]
pub struct BadFingerprint(String);

impl FromStr for Fingerprint {
    type Err = BadFingerprint;

    /// Accepts the prefix in any case or no prefix, and ignores `:`, spaces, and case
    /// in the digits, so that a copied fingerprint works.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || BadFingerprint(text.to_owned());
        let trimmed = text.trim();
        let digits = match trimmed.get(..7) {
            Some(prefix) if prefix.eq_ignore_ascii_case("sha256:") => &trimmed[7..],
            _ => trimmed,
        };
        let hex: Vec<u8> = digits
            .bytes()
            .filter(|b| !matches!(b, b':' | b' '))
            .collect();
        if hex.len() != 64 {
            return Err(bad());
        }
        let mut out = [0u8; 32];
        for (byte, pair) in out.iter_mut().zip(hex.chunks(2)) {
            let pair = std::str::from_utf8(pair).map_err(|_| bad())?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| bad())?;
        }
        Ok(Self(out))
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}
