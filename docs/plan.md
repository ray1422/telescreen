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
- ADR 0012 (user, 2026-11-06) replaces egui with a native UI on each platform: SwiftUI on macOS and Qt/QML on Linux. The view boundary stays. M20 changes the renderer contract and then removes egui and winit.

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
| M8 | v0.1.0 (internal milestone, no tag; user, 2026-10-27) | Support matrix in the spec updated; README build instructions | not started |
| M9 | `clipboard`: v0.2.0 text clipboard | The v0.2.0 conditions in spec section 2.1 pass on both directions (internal milestone, no tag) | code done (2026-10-27): core sync rules, NSPasteboard, wlr data-control; the backend starts on niri; the copy and paste test with the user did not run. CLIP-7 (concealed items with the mark, clear when the source clears) and CLIP-11 (clear at the session end and at switch off) code done (2026-10-27), automated tests pass on macOS and Linux; the user tested the copy and paste on the hardware with no failure (2026-10-27). done on the hardware |
| M10 | `pointer-dedup`: fix of the Alt+Tab bug (docs/test-plan.md section 6). The source sends a pointer datagram only if the position changed, and always when forwarding starts or the cursor enters the portal | An acceptance test: keys with no pointer motion send no pointer datagram; Alt+Tab on niri keeps its selection | code done, the acceptance test passes on macOS and Linux; the Alt+Tab check on niri waits for the user |
| M11 | `prompt-lost`: the prompt went away before the confirmation (user, 2026-10-27; not reproduced). One Linux to Mac session in the log ended with a heartbeat timeout after the prompt showed. The user thinks that a click out of the portal, or display sleep, can be the cause | A repro with the logs of both peers, then a decision with the user | not started |
| M12 | `hand-back`: v0.3.0. "Hand back" on the target, FLOW-24, the change of FLOW-18, the label "Disconnect", the verification model, and the operation drivers (spec section 10). The first release: tag `v0.3.0` (ask the user first) | The open points of hand back in spec section 10 are decided; acceptance tests pass, also with three peers for FLOW-24; a third real peer (headless sway in a container on the Linux computer, stub display helper, debug protocol) passes FLOW-24 (user, 2026-10-27); the hardware test passes in both directions | decided in spec sections 4.5 and 4.6 (ADR 0009, ADR 0010); the config contract is open |
| M13 | `clip-stream`: v0.4.0 separate clipboard stream (spec section 2.1). It sends images (PNG) and texts larger than 256 KiB (CLIP-6) (user, 2026-10-27) | To be specified | not started |
| M14 | `pointer-smooth`: the pointer on the target feels slow and not smooth (user, 2026-10-27; docs/test-plan.md section 6) | The loss, the latency, and the datagram interval are measured on the hardware; a fix is decided with the user and passes the same measurement | done (2026-10-28; docs/test-plan.md section 6): no loss. Mac to Linux: AWDL on the Wi-Fi of the Mac; Wi-Fi channel 149 or AWDL off removes it; README.md has the troubleshooting steps. Linux to Mac: datagrams waited in the event queue of the main loop; the target now injects only the newest position, and all pointer queues are bounded (NET-2). NET-8 and NET-9 measure it. No prediction (user, 2026-10-28) |
| M15 | `scroll-speed`: the scroll speed is adjustable (user, 2026-10-27) | `scroll-speed` in the computer section of the target (IN-14, user, 2026-11-04) | done |
| M19 | `kde-scroll`: Chrome animates the scroll on KDE, thus the scroll feels slow (user, 2026-11-05) | Button scroll on KWin (IN-15, spike 5); one wheel unit at each scroll end releases the scroll target (spike 10, 2026-11-08) | done; the user checked the pointer shape and the scroll feel with a real trackpad; the wheel unit: code complete, the hardware test did not run |
| M20 | `ui-redesign` (v0.600.0): spec section 4.7, ADR 0012, ADR 0013 (user, 2026-11-06) | The steps of M20 are done, and the user accepts the UI on macOS and on KDE | spec and ADRs done (2026-11-06); mockups continue |
| M21 | `focus-scroll`: in v0.5.0 the keys went to the target with no end after Mission Control, and one application did not scroll (user, 2026-11-07) | The native focus is the only keyboard focus state (FLOW-22); the target warns while the portal is not on screen, and shows the source cursor on the warning (FLOW-26); macOS suppresses forwarded buttons (IN-17); each forwarded scroll has an end (IN-18); AC-51 to AC-54 pass | code done (2026-11-07): tests pass on macOS and Linux; the hardware tests AC-51 to AC-54 are open |
| M22 | `arch-release`: the Arch package, the AppImage in the release, the applet in the Linux archives, the install methods of `install.sh` | DIST-6 to DIST-11: the `arch` jobs pass in CI; `install.sh` selects and installs each method on the Arch computer, interactive and with `--yes`; each user install has the applet; `README.md` has the options | code done (2026-11-10): the scripts, the workflows, and `telescreen self-check`. Tested in Docker on the Linux computer: the Arch package and the pacman install (also from `cat install.sh \| sh`), generic on Arch, native-user, the AppImage on Ubuntu 24.04, all checks failed, the change from a user install to native. Released as v0.504.0 (2026-11-10); the workflows passed, and a clean Arch container installed the published release with `curl | sh`. The user checked the applet in the KDE tray. Known gap: an old user install from install.sh before v0.504.0 has no `install-method`, so a change to native does not ask to remove it (the user decided not to fix it, 2026-11-10) |
| M23 | `reclaim` (v0.505.0): "Switch back" with the `reclaim` steps of a device (ACT-8, user, 2026-11-16) | The acceptance tests of ACT-8 pass on macOS and on the Linux computer; AC-60 and AC-61 pass on the hardware | code done (2026-11-17): the config contract, the controller, and the three UIs; the automated tests pass on both computers. Not done: AC-60 and AC-61 on the hardware; the user did not see the row in the three UIs |
| M24 | `login-stage` (v0.505.0): keys to the login screen of a Linux target (spec section 4.9, ADR 0015, user, 2026-11-16) | The acceptance tests of STG-1 to STG-11 pass on both computers; AC-62 to AC-66 pass on the hardware | code done (2026-11-17): the `stages` config, protocol 7, the stage sessions, the stage agent, `telescreen run --stage login` on Linux, the source flow, and the macOS UI; the automated tests pass on both computers; on the Linux computer the command refuses to start while a desktop session exists (STG-3). Hardware (user, 2026-11-17): AC-62 and AC-63 pass, after a fix that sends the waiting messages before a connection closes; AC-65 and AC-66 were seen during the tests. Not done: AC-64 on the hardware; the Qt UI and the Plasma applet do not show the login stage |

### M16: first binary release

v0.3.0 is the current code (user, 2026-11-04). Steps f and g of M12 move to a later version. See DIST-3.

Tested locally: the archive and `install.sh` on both computers (also a wrong checksum on macOS), and `makepkg` on Linux. `actionlint` reports no problem. The workflow itself did not run yet.

### M22: Arch package and the applet in the release

The Arch prebuild, the AppImage in the release, the applet in the Linux archives, and the install methods of `install.sh` (native, generic, appimage) (DIST-6 to DIST-11, user, 2026-11-09). Status: see the milestone table. The cleanup of the shell scripts kept the changes to `install.sh`, `package-release.sh`, `install-plasma-applet.sh`, and `AppRun`. The cleanup of `make-macos-app.sh`, `make-macos-control.sh`, and `packaging/appimage/build.sh` was not kept: it removed Info.plist keys and comments that explain the build.

### M17: start from a launcher

`telescreen` needs no argument, and the start window selects the computer (PEER-3, PEER-5, UI-8). The Linux archive has a launcher entry and a stripped binary (DIST-4). A local macOS build is not in `target/` (DIST-5). Tested on 2026-11-04 with the Mac and with KDE: the selection through the debug protocol on both, and a handoff from a portal on the external display to forwarding and a hand back. Not tested: a click in the KDE launcher.

### M18: more than one address and discovery

A computer can have a list of hosts, and a dial races them (NET-11). Discovery uses the system mDNS responder: "Allow discovery" for 10 minutes, and a search with each dial of the portal (NET-12 to NET-16, ADR 0011). Tested on 2026-11-05 between the Mac and Linux (KDE Plasma Wayland, Avahi), with the debug protocol and no clicks, also with a Mac app that `open` started: "Allow discovery" and "Refresh" connect in both directions, with an absent or a wrong host, and "Stop" removes the record. The 10-minute end and a Linux computer with no Avahi have only automated tests.

### M12 steps

Each step is one commit or more. Claude writes the tests first; Codex implements steps c to f.

| Step | Content | Status |
| --- | --- | --- |
| a | Spec sections 4.5 and 4.6, ADR 0009, ADR 0010, the glossary | done |
| b | The config contract: `docs/config.md`, the schema, the validator, the contract tests, the Rust config parser, and the example configs, in one change | done; `exec` and `prompt` return "not supported" until steps c and d |
| c | The driver runtime: `exec`, `command`, the search path | done (WIP on the `hand-back` branch) |
| d | The verifier runtime (JSON lines, `prompt`) and the device state on the destination | done (WIP on the `hand-back` branch) |
| e | Hand back on the wire and in the flows (HB-1 to HB-5, FLOW-17, FLOW-24) | done (WIP on the `hand-back` branch); FLOW-24 is tested with three controllers, step g adds a real third peer |
| f | The status icons (UI-4): macOS `NSStatusItem`, Linux StatusNotifierItem | replaced by M20 (UI-9 to UI-11) |
| g | The third peer for FLOW-24 (headless sway in a container) | not started |

### M20 steps

Order (user, 2026-11-07): the bug fixes come first. Do M21, then ACT-5 (the portal opens when a session starts and closes when it ends; done 2026-11-07: the UI has no "Open portal", and the portal shows one status line), then the update of the macOS UI to the current mockups (done 2026-11-07: the menu and the main window follow the canvas; "Add computer…" is disabled until the pairing of ADR 0014, and the gear button sends `open_settings`, which does nothing until UI-14), then step h (more than one portal), then the rest of step f1, f2, g, and i.

The sequence can change after the spikes.

| Step | Content | Status |
| --- | --- | --- |
| a | Spec section 4.7, ADR 0012, ADR 0013, the glossary | done (2026-11-06) |
| b | Mockups: the menu, the main window, the settings, the KDE applet, the control request flow, the icon | done (2026-11-07) |
| c | Spikes (ADR 0012, open points): the Wayland shortcut inhibit from a cxx-qt window on niri and KWin; `NSHostingView` and the C ABI with the Rust main loop | done (2026-11-07, spikes 6 and 7: KWin, nested niri, macOS); a supervised key test not run |
| d | Core: "Switch display only", the attach, and the control request on the wire (ACT-1 to ACT-5), with acceptance tests | done (2026-11-07): wire protocol 6, `SourceSession::attach`, and the new intents in the controller (`controller/actions.rs`); the current portal flow is not changed. Not tested on the hardware; no UI calls the intents yet (steps e to g) |
| e | View models: the menu and the main window, stable row ids, a serializable form, and the push renderer contract | done (2026-11-07): `MenuScreen`, `MainScreen`, `Publisher` in `telescreen-view` (UI-18, JSON shape in `tests/json.rs`); `menu_screen`, `main_screen`, `evaluate_displays`, `set_trusted` in the controller (`controller/menu.rs`) |
| f | macOS UI: Rust main loop, SwiftUI menu, status item, Control Center extension, main window, settings, lifecycle (UI-10, UI-13 to UI-16) | in progress: f1 (status item, menu, main window, reopen, Dock icon) done in `telescreen-ui-macos` (2026-11-07), checked with the bundle and the debug protocol, no clicks; winit still owns the run loop. f2 (2026-11-07): the Control Center button opens the menu through `telescreen://menu` (spike 8); `scripts/make-macos-app.sh` builds the extension. Not done: settings (UI-14), the single instance for a second CLI start, the on state of the button (UI-17). The custom SF Symbol is in the build; only a build with Xcode shows it |
| g | Linux UI: Qt/QML through cxx-qt, the Plasma applet, the StatusNotifierItem fallback, the menu command, lifecycle (UI-11, UI-13 to UI-16) | in progress (2026-11-07, on main; Qt 6.4 or newer, because the release builds on Ubuntu 24.04): the main window QML and the `Backend` bridge in `telescreen-ui-linux`, the applet in `applet/dev.telescreen.menu`, and `tools/fake_menu_service.py`; checked with fixtures on KDE Plasma 6.8 in Breeze and Breeze Dark, no clicks. g1a (2026-11-07): the app runs the Qt loop on the main thread and the winit loop on a second thread; the QML main window replaces the egui control window; a close hides it (UI-15). Built and tested on Linux with no window. g1b (2026-11-07): the D-Bus service of UI-11 (`MenuServer` in `telescreen-ui-linux`) and the single instance of UI-15; tested with direct calls of the interface object, not on a session bus. Not done: the StatusNotifierItem, `telescreen show menu`, the settings (UI-14) |
| h | More than one portal: one portal for each display (FLOW-2, FLOW-25). The controller keeps a portal for each display; the input capture watches more than one portal area and reports the portal of each event; the macOS and Linux captures and the portal windows follow | code done (on main, 2026-11-07): the core router, the capture API, the controller, and the window layer are per portal: one portal window for each display, and two portals can go to the same target. The macOS capture has 4 portal slots; the Linux capture has a state for each portal window and one key router. The menu and the main window have one session row for each portal, and `end_session` and `source_hand_back` name the portal. The automated tests pass on macOS and Linux. Not done: the hardware tests with two portals |
| i | Native portal windows; remove egui and winit | not started |

### M20 implementation notes

- From spikes 6 and 7 (2026-11-07):
  - A view model push is a full snapshot. SwiftUI skips old snapshots in a burst.
  - Rust sets the frame of each window. The SwiftUI view reports its size with `onGeometryChange`.
  - Menu panels need a subclass with `canBecomeKeyWindow = true`, else SwiftUI draws inactive controls.
  - Linux: drop all Wayland objects of Telescreen before `QGuiApplication`. Dispatch the private event queue from a Qt timer or a socket notifier.
  - Linux: the `wl_surface` comes from a private Qt API. Get it again for each inhibit request.
- From step d (2026-11-07):
  - Step e: the menu view model must show "Waiting for <peer>" for each control request that this peer sent. The controller has it only as `control_request_pending()`.
  - Steps f and g: call `evaluate_displays` at the start; call `set_trusted` with the local trust list (else every computer shows "Not trusted"); push `menu_screen` and `main_screen` through a `Publisher` after each event.
  - Steps f and g: the evaluation at the start runs the `portal-match` helpers of each display and dials the candidates. On macOS, a new install then shows the Local Network question at the start, not at the first portal.
  - Step f: a `rustc-link-arg` of a dependency does not reach the final binary. `telescreen-app/build.rs` sets the rpath to the Swift runtime. Without it, the bundle does not start ("Library not loaded: @rpath/libswift_Concurrency.dylib"), and macOS shows a dialog that tells the user to ask the developer.
  - Step f2: `scripts/make-macos-control.sh` makes the App Intents metadata with the Xcode tool, or copies the hand-written fallback in `crates/telescreen-ui-macos/control/fallback/` with a warning (user, 2026-11-07; spike 8 has the arguments). CI and the release set `TELESCREEN_REQUIRE_XCODE=1`. CI tested both tool calls on 2026-11-07 (Xcode 26.6 with the macOS 26.5 SDK): `actool` needs the baseline and the capline of the S, M, and L scales in the symbol template, and the generated metadata has no macOS 27 availability entry with that SDK. The custom SF Symbol (`tools/make_symbol.py`, `control/Assets.xcassets`) needs `actool` the same way: with no Xcode, the button shows the system symbol "display"; CI and the release require `actool`. The on state needs a shared state between the app and the sandboxed extension (an app group or a different channel); not decided.
  - Step f: the status glyph is the UI-17 glyph of the canvas (2026-11-07). `tools/make_icon.py` renders it with headless Chrome to `assets/macos/glyph-*.tiff`. `tools/make_symbol.py` writes the same glyph (the "menu-open" state) as the custom SF Symbol of the Control Center button, with the stroke of the stand as an outline.
  - Step f: on macOS the native main window replaces the egui control window. The start window (UI-8) and the verifier prompt (UI-5) stay egui until step i.
  - Step h: `control_request` and `control_match_done` in `controller/actions.rs` refuse with `Busy` if any source session exists. Change them to check only the matching display (ACT-6).
  - Step g, the plan (Claude decides, 2026-11-07):
  - g1: on Linux the Qt event loop runs on the main thread. The winit app (the controller, the portal windows, the egui start window and prompts) runs on a second thread with `EventLoopBuilderExtWayland::with_any_thread`, until step i removes winit. The QML main window replaces the egui control window. A close of the main window only hides it (UI-15). At quit, the winit thread stops and drops its Wayland objects first, then the Qt loop stops (spike 6).
  - g1a (Claude decides, 2026-11-07): the Qt loop starts only when the window layer starts the app, after the start window (UI-8). Thus an error at the start opens no Qt window. Rust requests to the Qt thread (show, close, quit) that come before QML makes the `Backend` wait, and the `Backend` runs them when it starts. If the Qt loop stops with no request from Rust, the app stops too.
  - g1: the D-Bus service of UI-11 (`dev.telescreen.Telescreen`, `/dev/telescreen/Menu`, `dev.telescreen.Menu1`) runs on its own zbus connection and thread. The single instance of UI-15 uses the same name: a second start calls `ShowMainWindow()` on the owner and stops.
  - g1b (Claude decides, 2026-11-07): `claim` in `telescreen-ui-linux` serves the object first and then requests the name, so that no method call comes before the object exists. The service has no sink until the app starts; the window layer then sets the sink and pushes `menu_screen` after each change, as the macOS menu. zbus runs the method calls on the executor thread of its connection.
  - g2: the StatusNotifierItem fallback and `telescreen show menu`. g3: the settings (UI-14), with step f.
- Step i: the verifier prompt window (UI-5) is egui too. Replace it before egui goes away.
  - Step i: the window layer must call `take_portal_window_request` also after network events. A received control request sets the request in `on_net`. Without that call, no portal window opens on the receiving peer, and the user cannot acknowledge.

- Corners (user, 2026-11-06): use continuous (G2) corners on all surfaces that Telescreen draws, on both platforms. The HTML mockups use circular corners; do not copy that.
  - macOS: SwiftUI gives G2 corners with `RoundedRectangle(cornerRadius:style: .continuous)`.
  - Qt Quick: `Rectangle.radius` draws circular arcs (G1). Make one shared QML component for a G2 rounded rectangle, for example a `QtQuick.Shapes` path with cubic corner curves, or a `ShaderEffect` with a superellipse distance function. Use it for all cards, buttons, and fields. Do not use `Rectangle.radius` directly.
  - Qt Quick: to clip child items to a G2 shape, use a layer mask (for example `MultiEffect` with `maskSource`). This costs one more render pass, so use it only where a child must be clipped.
  - KDE: the Plasma theme draws the frame of the applet popup. Telescreen cannot change those corners. G2 applies only to the items inside the popup.

## Not scheduled

The pairing UI is not scheduled. M20 makes the main window where it can go later:

- A pairing UI: spec section 4.8 (PAIR-1 to PAIR-8, ADR 0014, user, 2026-11-07), in a v0.6XX.0 version. Until then, pairing stays manual with `telescreen trust` (ADR 0004).
- A question on the first use of the clipboard sync (user, 2026-10-27). Until then, the switch starts on (CLIP-1).
- A setting "Start Telescreen at login" in the settings window (user, 2026-11-17). The settings window is not implemented (UI-14), and the repository installs no autostart file. Until then, the user configures the start at login by hand. The login stage needs it: after the login, the source waits until the Telescreen of the desktop session connects.
- Touchpad gestures to a Linux target (user, 2026-10-27). niri gets gestures only from libinput, which reads finger positions from a touchpad device. Thus the target needs a uinput multitouch touchpad, and the macOS source must read the finger positions (for example `NSTouch` in the portal). Telescreen can change the finger count, because niri cannot (niri: three fingers switch workspaces and scroll columns, four fingers toggle the overview). Example: macOS four fingers become Linux three fingers, and the macOS three-finger drag stays a left-button drag. Conflict: macOS uses four-finger swipes for Spaces, and a Space change makes the portal leave fullscreen (UI-3). Alternative: map a gesture to a shortcut, with no tracking of the fingers.
- Screen edges on KWin (user, 2026-11-04). The top-left hot corner does not open the overview through the uinput pointer, (issue 1). Probable cause, from the KWin 6.7 source (`Edge::check`): the first motion on an edge pushes the cursor back by 1 pixel, and the action needs a second motion on the edge 75 to 350 ms later. The source does not send a repeated position (NET-2). The kernel also does not send a repeated absolute axis value to the compositor. Thus the second motion does not arrive. Not confirmed with a measurement. Possible fix: the source sends a repeated position when it is on an edge of the portal, and the uinput pointer changes the value by 1 axis unit on each repeat.
- The scroll source (user, 2026-10-27). The target injects each scroll as a touchpad scroll (`finger`), so niri binds for the mouse wheel (`Mod+WheelScroll…`) do not apply. Send the source (wheel or touchpad) from the source computer, and inject a wheel with discrete steps. A touchpad scroll stays a touchpad scroll; niri `Mod+TouchpadScroll…` binds apply to it.

### Login screen control and the remote desktop portal (investigation, 2026-11-14)

These are roadmap items with no version and no sequence (user, 2026-11-14). The design is not started. Each item needs an ADR and a change to spec section 2, which lists login screen control and video streaming as out of scope. The memo in the open questions of `docs/spec.md` has the first notes.

Decisions of the user (2026-11-14):

- Roadmap item 1: the user operates the login screen of a Linux peer from the source. It is sufficient that the user can log in. The user must see the login screen; a procedure with no picture is not acceptable.
- Roadmap item 2: a handoff can open the portal as a remote desktop. The portal shows the picture of the physical display of the device. A remote virtual display is a different feature and is not part of this item.
- The peer does not use automatic login. KWallet must open at the login. Thus the password must go through PAM at the login screen.
- Key injection through uinput is accepted for the login screen.
- The feature can be a separate operation that relates to a device. It does not have to be part of the screen handoff.
- The config can get stages before the desktop session (login screen, later the boot loader). Not designed.
- Use an existing project where one fits. Do not write a new implementation of a part that a good project has.

Questions of 2026-11-14 and their answers:

- X11 greeter only, or a Wayland greeter too: not applicable to the first version, which has no picture. The uinput keyboard does not depend on the greeter.
- The identity and the trust keys before the login: the stage process runs with the account of the user, so it uses the usual files.
- The handover to the Telescreen of the desktop session: STG-9 and STG-11.
- The security rules for remote keys at a password field: STG-1 (`from`) and STG-5 (keys only, the target enforces it).
- The handoff confirmation before the login: no `verify`; the device state is unverified (STG-8).
- The test environment: the Linux test computer (user, 2026-11-17).

The measured facts of the Linux test computer (2026-11-14) are in the local hardware notes. The local hardware notes are not in the repository.

Decisions of the user for the first version (2026-11-16). They replace the items above where they differ:

- The first version is remote control of the login screen with keys only. The picture comes later.
- No service runs in the background. The user starts Telescreen on the target by hand when it is necessary, for example through SSH. It runs with the account of the user, so it reads the usual identity and trust keys. Thus the questions about the identity and the keys of a service before the login do not apply.
- The start command is `telescreen run --stage login` (the name is a proposal of Claude). It also makes the computer discoverable, and it prints a notice about this, because discovery is not a part of the login.
- Config: `stages` in the section of the computer that is controlled. The key is a built-in stage kind. Only `login` exists now, and a different key is an error. A computer has one stage of each kind at most. A stage has one key, `from`: the source computers that can control the stage, because a trusted fingerprint does not authorize (NET-4). Example: `stages: {login: {from: [mac]}}`. A stage has no `label` and no `input`: the display text and the permitted input are fixed for each kind. A stage belongs to a computer, not to a device. The desktop session is not a stage, and a config with no `stages` does not change. A later kind (for example a boot loader) is a new built-in key. The current stage is runtime state and is not in the config.
- The config has no hint text and no start command for the source. The source shows the command with a copy button. The text comes from Telescreen.
- The login stage has no `verify`. Forwarding starts after the connection, and the state is unverified (VER-2).
- Tentative: the login stage sends keys only, with no pointer and no clipboard.
- The control of a login stage also switches the display. The source runs its handoff steps for the device, then it opens the portal on that display, as "Switch & control". The portal takes the keys; there is no separate window for the keys. The user starts the action from the row of the target below a display, in the menu or in the main window, and that display gets the portal. The config does not relate a stage to a device.
- The name of the action is "Switch & log in" (user, 2026-11-16: "switch & login"; Claude writes the verb as "log in"). The item in the overflow menu has an ellipsis, because it opens the hint window. The button in the hint window and the row subtitle have the same name.
- The hint window has two states. First it shows the command and waits. When the target connects in the login stage, the wait line becomes a button. The display switches only after the user selects the button.
- After the login, the control continues with no new action of the user (see the states below). The source waits with no time limit, until the Telescreen of the desktop session connects or the user ends the session.
- The session of a login stage has "Switch back" (`reclaim`) and "End session". It has no "Hand back", because the target possibly cannot switch the display before the login.
- A new device key `reclaim` with `steps`: how this computer switches the device back to itself. The new action "Switch back" runs it. The other computer does not have to run Telescreen. On 2026-11-16 the user kept "Hand back" and `reclaim` separate. On 2026-11-17 the user replaced this: `reclaim` is the fallback of each hand back that is not possible (HB-4), and of the recovery when the source cannot send input (FLOW-27).
- Sequence that the user asked for: config, then mockups for the menu, then the implementation method, then the work.
- Decisions of 2026-11-17 (user): the first version is a macOS source and a Linux target. "Switch back" is v0.505.0 (ACT-8, in `main`), and the login stage was v0.506.0 (spec section 4.9, ADR 0015). Both are small versions. Later on that day the user put the two features in one version, v0.505.0. The test environment is the Linux test computer. Telescreen does not start at the login on that computer now; the user configures it.

States of the source for a login stage (proposal of Claude, 2026-11-16; the user agreed to the automatic continuation and to the wait with no time limit):

| State | What the user sees | Next state |
| --- | --- | --- |
| 1. Wait for the stage | The hint window with the command | The target connects in the login stage: 2 |
| 2. Stage connected | The hint window with the button | The user selects the button: 3 |
| 3. Control of the login screen | The portal is open. Keys only. The device state is unverified | The target reports the login: 4. A wrong password stays in 3 |
| 4. Wait for the desktop | The portal stays open. Keys stay on the source. The session card shows the wait | The Telescreen of the desktop session connects: 5 |
| 5. Usual control | The same as an attach ("Control only", ACT-3): the target grants the lease, resolves `input-target`, and runs its `verify` | The usual rules |

- Each state has "End session" and "Switch back". States 3 and 4 also end when the stage process stops, when the connection breaks, and with the safety exit (IN-8). Each end releases all held keys.
- The target finds the login from the login manager service of the system (logind): a new session of the user on the seat. Then it tells the source, releases the keys, and stops, so that the Telescreen of the desktop session can listen on the same port.
- The acknowledgement of the `prompt` verifier in state 5 is a key in the portal on the source (VER-4). Thus the user does not need the pointer on the target.
- The continuation is general: a stage ends, and the source waits for the same computer in its next state. A later boot loader stage can use it.

State after the implementation (2026-11-17; M23 and M24 in the table have the details):

- The mockups are in the design canvas of the user (the row for the login stage). The user selected the name "Switch & log in" and agreed to continue; the user did not see the built UI.
- The version number: v0.505.0 for the two features (user, 2026-11-17).
- Measured on the test computer (2026-11-17; the details are in the local hardware notes): a uinput keyboard of a process that the user starts through SSH reaches the login screen. logind sends `SessionNew` for the desktop login, and its properties (`Class=user` with a seat) separate it from an SSH login. Desktop programs start less than 1 second after that event. Thus the stage process cannot always free the port before the Telescreen of the desktop session starts: that Telescreen must be able to stop the stage process and to bind again.
- Not measured: keys with Shift and symbols on the login screen (the keyboard layout of the login screen), and the selection of the desktop session with keys.
- Run on the test computer (2026-11-17): `telescreen run --stage login` at the login screen. It listens, makes the keyboard, and publishes its record. The source connects to it when the source config has the address. The command stops at the desktop login, and the Telescreen of the desktop session gets the port.
- Open (2026-11-17): after a boot, the test computer at its login screen gets no mDNS answers from other computers, so the source does not find it by discovery. Discovery works some seconds after the first desktop login, and continues to work after a logout. The cause is not known. The system Avahi daemon is the only mDNS program before and after the login, and its log has no entry at the login. The user stopped the investigation (2026-11-17) and will use a fixed address. Thus the login stage must not depend on discovery; a possible design is that the stage process gets the address of the source from the SSH connection (not decided).

Findings from documents (not tested on the hardware):

- SDDM has no remote login and no remote control interface. A screen reader does not start on the SDDM greeter, so AT-SPI is not a reliable input path.
- Input: a uinput keyboard does not depend on the greeter (X11, Wayland, or a text console). XTEST works only on an X11 greeter.
- Picture of the login screen, by greeter:
  - X11 greeter: an X11 screenshot. The `Xsetup` hook of SDDM runs as root and can start a small capture program. Reports about x11vnc with SDDM say that it works but is not stable.
  - KWin greeter (SDDM in Wayland mode, Plasma Login Manager): the `org.kde.KWin.ScreenShot2` D-Bus interface. The caller needs an authorization. Its use on a greeter is not confirmed.
  - All greeters: DRM/KMS capture. It needs `CAP_SYS_ADMIN`, a cursor merge, and a read of the GPU buffer through EGL. The NVIDIA driver is the risk.
- A picture at 1 or 2 frames each second is sufficient for item 1. A stream is not necessary.
- Item 2 runs in the user session. The ScreenCast portal with PipeWire is the standard capture path there. It has no GPU buffer problem and needs no privilege. Not measured: the ScreenCast support of the portal backend on niri.
- Existing projects:
  - ReFrame (Apache-2.0): DRM/KMS capture and uinput, a privileged streamer process, an unprivileged VNC server, and a session helper. It lists remote login and NVIDIA. Its README does not name SDDM or KWin. It is the model for the process split.
  - kmsvnc (GPL-3.0): early development, one privileged process, no security. Not recommended.
  - Sunshine (GPL-3.0): runs only in the user session.
  - greetd: the login is an IPC protocol (`greetd-ipc(7)`), so a program can log in with no key injection and no picture. It replaces SDDM. The user did not decide on it.
- Virtual displays: EVDI does not build on kernel 7.1 and later, and a KWin 6.8.2 crash report names it. VKMS needs the compositor to use it as a second GPU. Neither is necessary for items 1 and 2.
- Before the operating system starts (firmware, GRUB): software cannot get input or a picture. The paths are a serial terminal of GRUB, a KVM-over-IP device, or a next-boot entry that the running system sets.
- Plasma Login Manager replaces SDDM in KDE from Plasma 6.6. Arch does not change the enabled display manager service; the user changes it.

## Rules for each milestone

- Write the acceptance tests first, with spec IDs in the test names or comments.
- One Codex task changes one responsibility. Give it the exact file allowlist.
- After each milestone: run all tests, update this table, and commit (clean, self-contained commits; see CLAUDE.md). Ask the user before a tag or a push.
- Do not claim a platform result that the user did not observe or that a test did not measure.
