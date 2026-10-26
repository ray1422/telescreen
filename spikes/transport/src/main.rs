//! SPIKE (throwaway): QUIC (quinn) between the Mac and the Linux computer.
//! Measures round trips on a reliable stream and on datagrams, and checks that the
//! client accepts only the server certificate with the pinned SHA-256 fingerprint.
//!
//!   spike-transport server <bind-addr>                  prints its fingerprint
//!   spike-transport client <server-addr> <fingerprint>  runs the measurement

use std::sync::Arc;
use std::time::{Duration, Instant};

use quinn::{ClientConfig, Endpoint, ServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};

const ALPN: &[u8] = b"telescreen-spike";
const N: usize = 1000;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Debug)]
struct Pinned(String, Arc<rustls::crypto::CryptoProvider>);
impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, ee: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let got = hex(&Sha256::digest(ee.as_ref()));
        if got == self.0 { Ok(ServerCertVerified::assertion()) } else {
            Err(rustls::Error::General(format!("fingerprint mismatch: got {got}")))
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.1.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.1.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.1.signature_verification_algorithms.supported_schemes()
    }
}

fn percentiles(name: &str, mut v: Vec<Duration>, sent: usize) {
    v.sort();
    let p = |q: f64| v[((v.len() as f64 - 1.0) * q) as usize].as_secs_f64() * 1000.0;
    println!("{name}: n={} lost={} p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms", v.len(), sent - v.len(), p(0.5), p(0.95), p(0.99), p(1.0));
}

async fn server(bind: &str) {
    let cert = rcgen::generate_simple_self_signed(vec!["telescreen".into()]).unwrap();
    let der = CertificateDer::from(cert.cert.der().to_vec());
    println!("fingerprint {}", hex(&Sha256::digest(der.as_ref())));
    let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let mut tls = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![der], key.into()).unwrap();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let cfg = ServerConfig::with_crypto(Arc::new(quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap()));
    let ep = Endpoint::server(cfg, bind.parse().unwrap()).unwrap();
    println!("listening on {bind}");
    while let Some(inc) = ep.accept().await {
        tokio::spawn(async move {
            let Ok(conn) = inc.await else { println!("handshake failed"); return };
            println!("connection from {}", conn.remote_address());
            let c2 = conn.clone();
            tokio::spawn(async move {
                while let Ok(d) = c2.read_datagram().await {
                    let _ = c2.send_datagram(d);
                }
            });
            while let Ok((mut tx, mut rx)) = conn.accept_bi().await {
                let mut buf = [0u8; 16];
                while rx.read_exact(&mut buf).await.is_ok() {
                    if tx.write_all(&buf).await.is_err() { break; }
                }
            }
            println!("connection closed");
        });
    }
}

async fn client(addr: &str, pin: &str) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone()).with_safe_default_protocol_versions().unwrap()
        .dangerous().with_custom_certificate_verifier(Arc::new(Pinned(pin.to_lowercase(), provider))).with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut ep = Endpoint::client("0.0.0.0:0".parse().unwrap()).unwrap();
    ep.set_default_client_config(ClientConfig::new(Arc::new(quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap())));
    let t = Instant::now();
    let conn = match ep.connect(addr.parse().unwrap(), "telescreen").unwrap().await {
        Ok(c) => c,
        Err(e) => { println!("connect failed: {e}"); return; }
    };
    println!("connected in {:.1}ms, max datagram {:?}", t.elapsed().as_secs_f64() * 1000.0, conn.max_datagram_size());

    // Reliable stream: 16-byte ping-pong at 100 Hz (like key events).
    let (mut tx, mut rx) = conn.open_bi().await.unwrap();
    let mut rtts = Vec::with_capacity(N);
    for i in 0..N as u64 {
        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(&i.to_le_bytes());
        let t = Instant::now();
        tx.write_all(&buf).await.unwrap();
        rx.read_exact(&mut buf).await.unwrap();
        rtts.push(t.elapsed());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    percentiles("stream RTT", rtts, N);

    // Datagrams at ~125 Hz (like pointer motion). Echoes after 200 ms count as lost.
    let mut rtts = Vec::with_capacity(N);
    let start = Instant::now();
    for i in 0..N as u64 {
        let mut buf = vec![0u8; 16];
        buf[..8].copy_from_slice(&i.to_le_bytes());
        buf[8..].copy_from_slice(&(start.elapsed().as_nanos() as u64).to_le_bytes());
        conn.send_datagram(buf.into()).unwrap();
        if let Ok(Ok(d)) = tokio::time::timeout(Duration::from_millis(200), conn.read_datagram()).await {
            let sent_ns = u64::from_le_bytes(d[8..16].try_into().unwrap());
            rtts.push(start.elapsed() - Duration::from_nanos(sent_ns));
        }
        tokio::time::sleep(Duration::from_millis(8)).await;
    }
    percentiles("datagram RTT", rtts, N);
    conn.close(0u32.into(), b"done");
    ep.wait_idle().await;
}

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("server") => server(&a[2]).await,
        Some("client") => client(&a[2], &a[3]).await,
        _ => eprintln!("usage: server <bind> | client <addr> <fingerprint>"),
    }
}
