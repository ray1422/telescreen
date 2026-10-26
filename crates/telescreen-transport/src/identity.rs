//! The key pair and certificate of this peer (ADR 0004). Each computer makes its
//! own at the first start. The files are in the local data directory, never in the
//! config (NET-5).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::Fingerprint;

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot make a key pair: {0}")]
    Generate(#[from] rcgen::Error),
}

pub struct Identity {
    pub certificate: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

impl Clone for Identity {
    fn clone(&self) -> Self {
        Self {
            certificate: self.certificate.clone(),
            key: self.key.clone_key(),
        }
    }
}

const CERT_FILE: &str = "certificate.der";
const KEY_FILE: &str = "private-key.der";

impl Identity {
    /// Makes a new self-signed identity in memory.
    pub fn generate() -> Result<Self, IdentityError> {
        let cert = rcgen::generate_simple_self_signed(vec!["telescreen".into()])?;
        Ok(Self {
            certificate: CertificateDer::from(cert.cert.der().to_vec()),
            key: PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()),
        })
    }

    /// Loads the identity from `dir`, or makes and saves a new one if there is none.
    /// The private key file has mode 0600.
    pub fn load_or_create(dir: &Path) -> Result<Self, IdentityError> {
        let cert_path = dir.join(CERT_FILE);
        let key_path = dir.join(KEY_FILE);
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| IdentityError::Io { path, source }
        };
        if cert_path.exists() && key_path.exists() {
            return Ok(Self {
                certificate: CertificateDer::from(fs::read(&cert_path).map_err(io(&cert_path))?),
                key: PrivatePkcs8KeyDer::from(fs::read(&key_path).map_err(io(&key_path))?),
            });
        }
        let identity = Self::generate()?;
        fs::create_dir_all(dir).map_err(io(dir))?;
        write_private(&key_path, identity.key.secret_pkcs8_der()).map_err(io(&key_path))?;
        fs::write(&cert_path, identity.certificate.as_ref()).map_err(io(&cert_path))?;
        Ok(identity)
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of(self.certificate.as_ref())
    }

    pub(crate) fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(self.key.clone_key())
    }
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(data)
}
