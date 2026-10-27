# Work plan to v0.2.0

This is the working plan. `docs/spec.md` is the specification. Update the status column when a milestone changes. Claude directs the work. Codex implements tasks through the `codex-delegate` skill. Hardware tests need the user (see `CLAUDE.local.md` for the rules of the Linux test computer).

## Crates

One Cargo workspace under `crates/`. Dependencies point downward only. No crate below `telescreen-app` contains platform code except the `platform-*` crates.

| Crate | Owns | Does not own |
| --- | --- | --- |
| `telescreen-core` | config parse and semantic validation (parity with `tools/validate_config.py`), the domain model (peer, device, session, lease), the session state machine (FLOW-*), the wire messages and their encoding, key identity and the macOS-to-evdev key table, pointer mapping (IN-11), the platform traits (capture, inject, display switch, prompt), the `command` driver runner | I/O threads, sockets, windows, OS APIs |
| `telescreen-transport` | QUIC (quinn) peer endpoint: dialer and listener, mutual pinning (ADR 0004), the trust store and the key pair files, reliable stream and pointer datagrams with sequence numbers, heartbeat | session rules (it moves messages only) |
| `telescreen-platform-macos` | event tap capture and suppress, safety exit counter, watchdog, `CGEventPost` injection, portal window AppKit adjustments (level, input sources), prompt window, Secure Event Input detection | session rules |
| `telescreen-platform-linux` | wlr virtual pointer and uinput keyboard injection, layer-shell prompt, portal window input, shortcut inhibit | session rules |
| `telescreen-view` | view models (what each screen shows: portal, prompt, control window) and intents (what the user did). Plain data, no UI library | drawing, windows, OS APIs |
| `telescreen-render-egui` | the egui renderer: it draws a view model on a winit window and returns intents | session rules, window placement, fullscreen, window level, shortcut inhibit |
| `telescreen-app` | the `telescreen` binary: CLI (`run`, `trust`, `fingerprint`, `check-config`, `displays`), winit windows (placement, fullscreen, level through the platform crates), wiring of core + transport + platform + view + renderer; the macOS `.app` bundle script | domain logic |

### View boundary

The user plans to replace the UI library later (user, 2026-10-27). Thus:

- Only `telescreen-render-egui` uses egui. No other crate imports egui, egui-winit, or egui-wgpu.
- The app makes a view model from the session state and gives it to the renderer. The renderer returns intents. The app does not read the state of the renderer.
- Window management (winit windows, fullscreen, window level, shortcut inhibit, input sources) is not part of the renderer. A different renderer uses the same windows.
- The Linux layer-shell prompt draws the same prompt view model with its own small text renderer in `telescreen-platform-linux`.

## Who does what

- Claude: crate skeletons, public traits and types, `Cargo.toml` dependencies, acceptance tests, platform code that needs hardware tests, review, commits, hardware test sessions with the user.
- Codex (`gpt-6.1-sol`, `high`): implementations behind tested interfaces. Good candidates: config parsing and validation, wire encoding, the session state machine, the key table, pointer mapping, the trust store, transport framing. Permit new unit tests (`--allow-new-tests`) for tasks with internal logic.
- The user: hardware tests, permissions (Accessibility, Input Monitoring, Local Network), decisions on open questions.

## Milestones

| ID | Milestone | Done when | Status |
| --- | --- | --- | --- |
| M0 | Workspace skeleton, traits, error types, CI-like local check script | `cargo check --all-targets` passes on macOS; `cargo check` for Linux crates passes on the Linux computer | done (2026-10-27): `cargo check` passes on macOS and on the Linux computer |
| M1 | Config in Rust (`telescreen-core`) | Rust tests mirror `tools/test_config.py` and pass; `telescreen check-config examples/config.yaml` works | done (2026-10-27): `tests/config.rs` passes; YAML 1.1/1.2 ambiguity rule added to `docs/config.md` |
| M2 | Core session model: state machine, wire messages, key table, pointer mapping | Unit and acceptance tests for FLOW-5..FLOW-19, IN-3..IN-5, IN-11, NET-2, NET-6 pass with fake platforms and a fake transport | done (2026-10-27): with fake platforms; tentative protocol in ADR 0007 |
| M3 | Transport | Two peers on one machine connect with mutual pinning; an unknown key is refused on both sides (AC-21); pointer datagrams drop old sequence numbers | done (2026-10-27): loopback tests, including the heartbeat timeout of a hung peer |
| M4 | Linux target | With a test source over the LAN: pointer and keys inject on niri, the prompt shows with the hint (user watches; no keys or clicks without supervision) | code done; hardware test not run (the Linux computer was busy) |
| M5 | macOS source | Portal Space, confirmation, capture, switch of the controlled computer at the built-in display edge, focus lock, release shortcut, safety exit, watchdog | code done; hardware test not run |
| M6 | Mac to Linux end to end | AC-1..AC-21, AC-23..AC-25 pass on the hardware for this direction | not started (needs the user) |
| M7 | Linux source and macOS target | AC-22 passes; the reverse session works (AC-23) | code done (Linux portal capture, macOS injection and prompt); hardware test not run |
| M7a | Keyboard focus model, portal guard, target actions (FLOW-17, FLOW-21..FLOW-23, IN-13, UI-7) | AC-23, AC-26..AC-29 pass on the hardware | code done (2026-10-27); with the debug protocol and no clicks or keys: both directions reach forwarding, "Control from here" and "Stop this session" work, and the focus holder takes and returns the keyboard focus on niri and on macOS (Telescreen frontmost); clicks and keys not tested |
| M8 | v0.1.0 | Support matrix in the spec updated; README build instructions; tag `v0.1.0` (ask the user first) | not started |
| M9 | `clipboard`: v0.2.0 text clipboard | The v0.2.0 conditions in spec section 2.1 pass on both directions; tag `v0.2.0` (ask the user first) | code done (2026-10-27): core sync rules, NSPasteboard, wlr data-control; the backend starts on niri; the copy and paste test with the user did not run. CLIP-7 (concealed items with the mark, clear when the source clears) and CLIP-11 (clear at the session end and at switch off) code done (2026-10-27), automated tests pass on macOS and Linux; not tested on the hardware |
| M10 | `pointer-dedup`: fix of the Alt+Tab bug (docs/test-plan.md section 6). The source sends a pointer datagram only if the position changed, and always when forwarding starts or the cursor enters the portal | An acceptance test: keys with no pointer motion send no pointer datagram; Alt+Tab on niri keeps its selection | not started; do it before any tag |
| M11 | `prompt-lost`: the prompt went away before the confirmation (user, 2026-10-27; not reproduced). One Linux to Mac session in the log ended with a heartbeat timeout after the prompt showed. The user thinks that a click out of the portal, or display sleep, can be the cause | A repro with the logs of both peers, then a decision with the user | not started |
| M12 | `hand-back`: v0.3.0. "Hand back" on the target, FLOW-24, the change of FLOW-18, and the label "Disconnect" (spec section 10) | The open points of hand back in spec section 10 are decided; acceptance tests pass, also with three peers for FLOW-24; a third real peer (headless sway in a container on the Linux computer, stub display helper, debug protocol) passes FLOW-24 (user, 2026-10-27); the hardware test passes in both directions | design open |
| M13 | `clip-stream`: v0.4.0 separate clipboard stream (spec section 2.1). It sends images (PNG) and texts larger than 256 KiB (CLIP-6) (user, 2026-10-27) | To be specified | not started |

## Rules for each milestone

- Write the acceptance tests first, with spec IDs in the test names or comments.
- One Codex task changes one responsibility. Give it the exact file allowlist.
- After each milestone: run all tests, update this table, and commit (clean, self-contained commits; see CLAUDE.md). Ask the user before a tag or a push.
- Do not claim a platform result that the user did not observe or that a test did not measure.
