# Work plan to v0.2.0

This is the working plan. `docs/spec.md` is the specification. Update the status column when a milestone changes. Claude directs the work. Codex implements tasks through the `codex-delegate` skill. Hardware tests need the user (see `CLAUDE.local.md` for the rules of the Linux test computer).

## Crates

One Cargo workspace under `crates/`. Dependencies point downward only. No crate below `telescreen-app` contains platform code except the `platform-*` crates.

| Crate | Owns | Does not own |
| --- | --- | --- |
| `telescreen-core` | config parse and semantic validation (parity with `tools/validate_config.py`), the domain model (peer, device, session, lease), the session state machine (FLOW-*), the wire messages and their encoding, key identity and the macOS-to-evdev key table, pointer mapping (IN-11), the platform traits (capture, inject, display switch, prompt), the `command` driver runner | I/O threads, sockets, windows, OS APIs |
| `telescreen-transport` | QUIC (quinn) peer endpoint: dialer and listener, mutual pinning (ADR 0004), the trust store and the key pair files, reliable stream and pointer datagrams with sequence numbers, heartbeat | session rules (it moves messages only) |
| `telescreen-platform-macos` | event tap capture and suppress, safety exit counter, watchdog, `CGEventPost` injection, cursor freeze and edge release, portal window AppKit adjustments (level, input sources), prompt window, Secure Event Input detection, the disclaim spawn of the network helper (ADR 0005) | session rules |
| `telescreen-platform-linux` | wlr virtual pointer and uinput keyboard injection, layer-shell prompt, portal pointer lock, relative pointer, shortcut inhibit | session rules |
| `telescreen-app` | the `telescreen` binary: CLI (`run`, `trust`, `fingerprint`, `check-config`), winit + egui portal UI, wiring of core + transport + platform; on macOS also the `telescreen-net` binary and the `.app` bundle script | domain logic |

## Who does what

- Claude: crate skeletons, public traits and types, `Cargo.toml` dependencies, acceptance tests, platform code that needs hardware tests, review, commits, hardware test sessions with the user.
- Codex (`gpt-6.1-sol`, `high`): implementations behind tested interfaces. Good candidates: config parsing and validation, wire encoding, the session state machine, the key table, pointer mapping, the trust store, transport framing. Permit new unit tests (`--allow-new-tests`) for tasks with internal logic.
- The user: hardware tests, permissions (Accessibility, Input Monitoring, Local Network), decisions on open questions.

## Milestones

| ID | Milestone | Done when | Status |
| --- | --- | --- | --- |
| M0 | Workspace skeleton, traits, error types, CI-like local check script | `cargo check --all-targets` passes on macOS; `cargo check` for Linux crates passes on the Linux computer | not started |
| M1 | Config in Rust (`telescreen-core`) | Rust tests mirror `tools/test_config.py` and pass; `telescreen check-config examples/config.yaml` works | not started |
| M2 | Core session model: state machine, wire messages, key table, pointer mapping | Unit and acceptance tests for FLOW-5..FLOW-19, IN-3..IN-5, IN-11, NET-2, NET-6 pass with fake platforms and a fake transport | not started |
| M3 | Transport | Two peers on one machine connect with mutual pinning; an unknown key is refused on both sides (AC-21); pointer datagrams drop old sequence numbers | not started |
| M4 | Linux target | With a test source over the LAN: pointer and keys inject on niri, the prompt shows with the hint (user watches; no keys or clicks without supervision) | not started |
| M5 | macOS source | Portal Space, confirmation, capture, pause and resume at the built-in display edge, focus lock, release shortcut, safety exit, watchdog | not started |
| M6 | Mac to Linux end to end | AC-1..AC-21, AC-23..AC-25 pass on the hardware for this direction | not started |
| M7 | Linux source and macOS target | AC-22 passes; the reverse session works (AC-23) | not started |
| M8 | v0.1.0 | Support matrix in the spec updated; README build instructions; tag `v0.1.0` (with the user's approval) | not started |
| M9 | v0.2.0 text clipboard | The v0.2.0 conditions in spec section 2.1 pass on both directions; tag `v0.2.0` (with the user's approval) | not started |

## Rules for each milestone

- Write the acceptance tests first, with spec IDs in the test names or comments.
- One Codex task changes one responsibility. Give it the exact file allowlist.
- After each milestone: run all tests, update this table, and ask the user before the commit.
- Do not claim a platform result that the user did not observe or that a test did not measure.
