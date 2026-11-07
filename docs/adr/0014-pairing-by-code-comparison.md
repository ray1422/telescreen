# 0014: Pairing by code comparison

Status: accepted for a v0.6XX.0 version; the version number is not decided (user, 2026-11-07). Not implemented.

## Context

ADR 0004 makes the user type the fingerprint of each peer with `telescreen trust`. This is too much work (user, 2026-11-07). The main window was going to accept a typed fingerprint (UI-13), and the user withdrew that for the same reason.

The user wants (2026-11-07):

- a pairing flow in the GUI and in the CLI;
- the CLI on B, also over SSH from A, so that the display does not need to show B first;
- a confirmation on A;
- no typing, and one human action on each side.

A PIN alone is not sufficient: a person who sees the PIN can send a request first (user).

## Decision

- Numeric comparison with a commitment (spec PAIR-4). The connection uses the long-term keys. The 6-digit code binds the fingerprints of both peers.
- B shows the code. A shows 3 codes and no "Accept" button, and the user selects the code that B shows (spec PAIR-2). Thus a request that the user did not start on B cannot be accepted blindly.
- A temporary port and an mDNS record with a random instance name find the peer. The record is not trusted (spec PAIR-3).
- A second connection during pairing stops the pairing. Pairing does not replace a fingerprint in the trust list (spec PAIR-5).
- `telescreen pair` runs the same flow with no running app (spec PAIR-6).
- `telescreen trust` stays as the fallback (ADR 0004).

The details that the user did not state one by one are marked "Claude decides" in spec section 4.8.

## Consequences

- This is not trust on first use: the user compares the code.
- An attacker in the middle has 1 chance in 10^6 for each attempt, and the user sees each failed attempt.
- The host name and the pairing state are visible on the LAN. The user accepts this (2026-11-07).
- Open: `--host` when mDNS does not work, and a PIN as an option (spec PAIR-8).
