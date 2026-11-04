# 0011: Discovery uses the system mDNS responder and is only a hint

Status: accepted (user, 2026-11-05).

## Context

A computer can get a new IP address, and the config holds a fixed host. Discovery must find a trusted peer without a change to the identity model.

A question in mDNS is visible to all computers on the network. Any computer can copy it. Thus a question cannot prove the identity of the sender. A challenge would make the responder show that it exists. The listener on the endpoint port also answers each TLS handshake with its certificate, so an active scan already finds the computer.

## Decision

- Discovery gives an address. Only mutual TLS (NET-3) gives an identity.
- Telescreen uses the responder of the system: Bonjour on macOS, Avahi through D-Bus on Linux. It has no responder of its own and no fallback.
- A computer is discoverable only for 10 minutes after the user selects "Allow discovery". The user searches with "Refresh". Telescreen does not search or announce without these buttons.
- The record has a random instance name and the fingerprint in clear text as the TXT key `fp`.

## Consequences

- A false record causes at most one failed dial.
- In the 10 minutes, a passive observer on the network can read the fingerprint and the host name, and can link them over time. The fingerprint does not contain a name.
- A computer with no system responder has no discovery. Dials to configured hosts still work.

## Later goal

The user wants no stable identity on the network (user, 2026-11-05). This needs a change to pairing: a shared secret, a changing tag in the TLS ClientHello, and a listener that sends no certificate if the tag does not match. The discovery record can then use the same secret. This replaces ADR 0004 and changes the protocol version. A tag can be replayed for a short time. This goal is not part of this decision.
