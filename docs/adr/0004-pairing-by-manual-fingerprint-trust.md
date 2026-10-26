---
status: accepted
---

# v0.1.0 pairing: manual fingerprint trust

On the first start, each computer makes its own key pair and shows its fingerprint. The user adds the fingerprint of each peer with `telescreen trust <computer-id> <fingerprint>`. The local trust list is in the local data directory, not in the config. Both peers present a certificate in each connection (QUIC with mutual TLS, ADR 0001). The dialer and the listener each accept the connection only if the key of the other peer is in the local trust list for that computer ID. If it is not, refuse the connection. Spike 4 verified only the dialer side; the product must verify both sides. v0.1.0 has no trust on first use and no pairing-code UX.
