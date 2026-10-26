# Telescreen specification

Status: draft for v0.1.0, 2026-10-27. This file is the normative product specification. `docs/config.md` and `schema/config.schema.json` specify the config and the driver contract. `GLOSSARY.md` defines the terms. `docs/adr/` records why each decision was made. `docs/spikes.md` and `docs/hardware.md` record measured facts.

Requirement IDs (for example `FLOW-3`) are stable. Refer to them in tests, tasks, and reviews. "Must" is a requirement. "Should" is a strong recommendation. "Can" is a permission.

## 1. Purpose

Some computers share physical devices, for example one display that has video cables from more than one computer. Telescreen lets the user control these computers with one keyboard and one mouse. The user opens a portal on a computer, selects a different computer, and Telescreen switches the shared display to that computer. Then the keyboard and the mouse of the first computer control the second computer while the pointer is in the portal. The picture goes through the video cable. Telescreen does not send video.

Example setup: a MacBook with a built-in display and an external display. A Linux computer also connects to the external display. The keyboard and the mouse are on the Mac.

## 2. Scope

### 2.1 Versions

Versions follow Semantic Versioning. There is no stable release, so all versions are 0.x. In 0.x, a new feature increases the minor version, and a fix increases the patch version.

| Version | Content |
| --- | --- |
| v0.1.0 | Display handoff, confirmation prompt with the hint, and keyboard and mouse forwarding, in both directions: macOS to Linux, and Linux to macOS. |
| v0.2.0 | Opt-in text clipboard sync. It syncs only in an active, authenticated session, with no loops between the peers, and it sends nothing after the user disables it or after a disconnect. |
| v0.3.0 (candidate) | Image clipboard (PNG), sent only when the other peer pastes, with a "Pasting from <label>…" overlay. |

### 2.2 Not part of v0.1.0

Video streaming, clipboard (see 2.1), file transfer, atomic switching of device groups, arbitrary remote shell commands, interception of all system keys on every OS, login screen or secure desktop control, emulated displays, game relative-mouse mode, cross-platform IME translation, shortcut remapping (for example Command+C to Ctrl+C), NAT traversal, Windows.

### 2.3 General constraints

- **GEN-1** You must not hard-code macOS, Linux, two computers, or one display. Any number of computers and displays is possible. Only some computers can connect to a given device.
- **GEN-2** A device is not always a display. Any equipment that configured operations can switch is a device.
- **GEN-3** You must not claim support for a platform, a compositor, or a shortcut that you did not test. Record each verified environment in the support matrix (section 8).
- **GEN-4** Do not make up benchmark numbers.

## 3. Peers and roles

- **PEER-1** Each computer runs the same Telescreen. Each peer can be the source and the target of sessions (ADR 0006). There is no server and no client.
- **PEER-2** Each peer listens on the `endpoint` of its computer in the config. The source dials the target.
- **PEER-3** The user selects the local computer ID at startup. Telescreen must not guess it from the host name.
- **PEER-4** Each peer reads the same config format. Each peer runs only the operations of its own computer (`docs/config.md`).

## 4. User flow

### 4.1 Portal and candidates

- **FLOW-1** Read and validate the config. Detect the local capabilities (input capture, injection, permissions).
- **FLOW-2** The user opens a portal. The portal is not in the config. The portal opens on the display of the control window (user, 2026-10-27). To use a different display, move the control window to it first. On the display of the portal, evaluate `portal-match` of the local devices. No match: tell the user. More than one match: the user selects one. Do not use the first match silently.
- **FLOW-3** Make the candidate targets: the local `handoff` keys that are also computers with the same device `id`. Show a candidate that is offline, that has no permission, or that has no `input-target` as unavailable, with the reason.
- **FLOW-4** When the portal moves to a different display, or the display topology changes, evaluate `portal-match` again. The previous device binding and its coordinates must not continue to apply.

### 4.2 Handoff and confirmation

- **FLOW-5** After the user selects a target, the target resolves its local `input-target` by the shared device `id`, confirms that it can inject input, and gives an exclusive lease for that device to this session.
- **FLOW-6** The source stops any old input stream and releases all held keys and buttons. Then it runs the handoff `steps`, and `verify` if it is present, on the source computer.
- **FLOW-7** After the handoff, the target shows the prompt on its input-target display (ADR 0002). The prompt shows the `hint` of the target computer, if it is present, as plain text.
- **FLOW-8** The user confirms with the confirmation key (Return. In v0.1.0 the key is fixed; a later version makes it configurable, user, 2026-10-27), or with a click in the portal. The source accepts the confirmation only while the target reports that the prompt is visible, and the portal has the window focus. At other times the key is a normal key.
- **FLOW-9** The source suppresses the confirmation key and the confirmation click. It does not forward them. The click on the switch button must not go to the target.
- **FLOW-10** Forwarding starts only after the confirmation, and only if the display token of the target is still valid. After the confirmation, the UI shows the video input as confirmed.
- **FLOW-11** If there is no confirmation in 60 seconds (fixed in v0.1.0; configurable in a later version, user, 2026-10-27), cancel the wait. The target removes the prompt. The UI shows the video input as unknown. Do not switch the video input back.
- **FLOW-12** If a handoff step fails, or `verify` is false: do not start forwarding, do not retry automatically, and do not claim a rollback. After a partial success, show the video input as unknown.

### 4.3 Forwarding, pause, and end

- **FLOW-13** While the pointer is in the portal, the source captures, suppresses, and forwards its keyboard and mouse input to the target. The input focus follows the portal region. The source must not need window focus events from the target.
- **FLOW-14** Pause: when the pointer leaves the portal through an edge that touches a different local display, input goes back to the source, and the session stays (ADR 0003). The target display does not change. When the pointer comes back into the portal, forwarding starts again with no new confirmation, if no new handoff ran.
- **FLOW-15** Portal edges with no adjacent local display stop the pointer.
- **FLOW-16** These events end the session: the release shortcut, the safety exit, a disconnect (heartbeat timeout or a closed connection), "disconnect" on the target, a request from the target to get control back, lease expiry, removal of the portal display, and revocation of a permission. Then the source stops suppression and restores the cursor and the key state. The target releases all held keys and buttons. If the video input is not confirmed, show it as unknown. Do not claim an automatic recovery.
- **FLOW-17** If the target requests control back, it ends the session. Then it can start a new session as the source in the other direction.
- **FLOW-18** A session end does not switch the video input back. A switch back needs an explicit handoff route. The user should be able to start the switch back from a display of the source that the handoff does not change (for example the built-in display).
- **FLOW-19** If the portal display disappears from the source, stop forwarding and restore the UI. Do not continue to suppress input in a window that moved to a different display.

## 5. Input

- **IN-1** Capture, suppress, forward, and inject are four separate capabilities. Detect and report each one. Window key events and global hotkey registration are not input capture.
- **IN-2** Input callbacks (event taps, Wayland input events) only capture, decide to suppress, and push to a bounded queue. They must not run helpers, wait for the network, or block the UI.
- **IN-3** Key and button down/up events must stay in pairs. Pointer motion can be merged. Key and button events must not be dropped. If the queue overflows, end the session and release all held state.
- **IN-4** Both peers release all held keys and buttons on a session end. The target must release them on a heartbeat timeout by itself; it must not wait for key-up events from the source.
- **IN-5** Keys go by physical key identity, not by the character. The input method of the source must not change the key (spike 1). Modifier keys go by physical key: Command to Super, Option to Alt, Control to Control.
- **IN-6** Do not remap shortcuts. The `hint` tells the user the shortcuts of the target.
- **IN-7** The release shortcut (macOS: Control+Option+Command+Escape. Linux: Control+Alt+Shift+Escape, user, 2026-10-27. Fixed in v0.1.0; configurable in a later version) ends the session. It is never forwarded.
- **IN-8** The safety exit (Escape 10 times in 5 seconds) ends suppression and the session. The input callback counts the presses itself, with no dependency on the main thread, the UI, or the network. Escape is still forwarded.
- **IN-9** A watchdog that is independent of input events must stop suppression when the main loop does not respond for a few seconds, and when a suppression deadline passes (spike 2).
- **IN-10** While forwarding on macOS, suppress all gestures and shortcuts that switch Spaces or applications, or that open Mission Control. Do not forward them (ADR 0003). When the pointer is out of the portal, they go to macOS.
- **IN-11** Pointer position: the source maps the pointer position in the portal, normalized to the portal size, to the input-target display of the target. The backends handle scale, rotation, negative origins, and pixel coordinates. The pointer must not leave the input-target display on the target.
- **IN-12** If the OS blocks capture (for example macOS Secure Event Input, or a missing permission), report it clearly. Do not claim that capture works.

## 6. Portal window and prompt

- **UI-1** The portal is borderless and covers its display. On macOS it is a native fullscreen Space, above the menu bar level (window level 26), so the menu bar and the title bar do not show at the top edge (ADR 0003, spike 1).
- **UI-2** While the portal has the focus, it allows only Roman input sources, and macOS restores the previous input source after it (spike 1).
- **UI-3** If the portal leaves fullscreen (for example a drag in Mission Control), detect it and pause forwarding. Tentative (proposed by Claude, not confirmed by the user): enter fullscreen again.
- **UI-4** The portal shows the candidate targets, the state of the session (waiting for confirmation, forwarding, paused, ended), and the video input state (confirmed, unverified, unknown).
- **UI-5** The prompt on the target is an overlay on the input-target display. On Linux use layer-shell where the compositor has it; else a normal fullscreen window.
- **UI-6** On Linux as the source, the portal is a fullscreen window. While forwarding it locks the pointer, reads relative motion, and inhibits compositor shortcuts (ADR 0006). To pause at an edge, it unlocks the pointer with a position hint at that edge. Tentative (proposed by Claude; the user asked about portability and did not reject it).

## 7. Network and security

- **NET-1** The transport is an abstraction with a reliable ordered channel (control messages, key and button events) and an unreliable channel for the latest pointer position (ADR 0001). Key and button events must use the reliable channel.
- **NET-2** v0.1.0 uses QUIC: reliable streams for the reliable channel, datagrams for the pointer. Each pointer datagram has a sequence number. The target drops a datagram that is older than the last one that it used.
- **NET-3** Each peer has its own key pair. Both peers present a certificate, and each peer accepts the other only if its fingerprint is in the local trust list for that computer ID (ADR 0004). The user adds a fingerprint with `telescreen trust <computer-id> <fingerprint>`. No trust on first use.
- **NET-4** A peer must not accept an injection request only because the sender is on the LAN. A trusted fingerprint identifies a peer; it does not authorize it. The target refuses a session request with `NoRoute` if its config has no handoff route from that source to the device (user, 2026-10-27).
- **NET-5** The config does not hold keys, the trust list, or runtime state.
- **NET-6** Sessions have a generation number and a heartbeat. A lease is held by one session only. The local state of one peer alone must not decide the lease owner.
- **NET-7** On macOS, the network runs in the app process (ADR 0008). The app keeps `NSLocalNetworkUsageDescription` in `Info.plist`. The Local Network permission of the installed app is open (AC-25).

## 8. Platforms

### 8.1 Backends for v0.1.0

| Capability | macOS | Linux (Wayland) |
| --- | --- | --- |
| Portal window | winit native fullscreen + AppKit window level | winit fullscreen window |
| Capture and suppress | CGEventTap at the HID location (Accessibility, Input Monitoring) | Pointer lock + relative pointer + keyboard shortcuts inhibit in the portal |
| Pointer injection | `CGEventPost` | wlroots virtual pointer, absolute motion on the input-target output |
| Key injection | `CGEventPost` | uinput keyboard (the compositor keymap applies) |
| Prompt overlay | AppKit window above the fullscreen level | layer-shell overlay |
| Network | in process (ADR 0008) | in process |

### 8.2 Support matrix

Record each environment that passed the acceptance criteria. At this time only spikes ran (`docs/spikes.md`):

| Environment | Status |
| --- | --- |
| macOS 27.0.1, Apple silicon | Spikes 1, 2, 4 passed. Product not tested. |
| Linux, niri 26.04 (Wayland), x86_64 | Spike 3 (pointer) passed. Product not tested. Keyboard injection, layer-shell prompt, and capture not tested. |
| Linux X11, KDE, GNOME, sway, Hyprland, Windows | Not tested. Do not claim support. |

### 8.3 Distribution

- **DIST-1** v0.1.0 is built from source: a script makes an ad-hoc signed `Telescreen.app` on macOS, and `cargo install` on Linux.
- **DIST-2** Before a binary release: test the real download path (dmg with quarantine, Gatekeeper, App Translocation) and each signing type that you ship (`docs/spikes.md`, spike 4).

## 9. Acceptance criteria

Each row must pass on each environment in the support matrix before you mark a version complete.

| ID | Scenario | Expected |
| --- | --- | --- |
| AC-1 | Desktop / Monitor1 / MyScreen use the same device `id` | Telescreen finds the same physical device. It does not compare the aliases |
| AC-2 | Two Linux computers share a second display. The Mac does not connect to it | The Mac is not a candidate for the second device |
| AC-3 | Select the Linux computer in the portal on the external display | The handoff runs on the Mac. The display shows the Linux computer. The prompt shows on the input-target display with the hint |
| AC-4 | Click the switch button | The click does not go to the target |
| AC-5 | Press the confirmation key in the focused portal while the prompt shows | Forwarding starts. The key does not go to the target. The video input shows as confirmed |
| AC-6 | Press the confirmation key with no prompt, or with no portal focus | It is a normal key. Forwarding does not start |
| AC-7 | No confirmation in the timeout | The wait stops. The prompt goes away. The video input shows as unknown. It does not switch back |
| AC-8 | The portal moves to a different display | `portal-match` runs again. The old binding and coordinates do not apply |
| AC-9 | `portal-match` false / more than one true / timeout | No match / ambiguous / error. No device is selected arbitrarily |
| AC-10 | A handoff step fails, or `verify` is false | Forwarding does not start. No retry. No rollback claim |
| AC-11 | The pointer leaves the portal to the built-in display, then comes back | Input is local while out; the session stays; forwarding starts again on return with no confirmation. No key or button stays pressed on the target |
| AC-12 | Resolution, scale, rotation, negative coordinates, different aspect ratios | The target pointer position is correct. Out-of-bounds behavior is defined |
| AC-13 | Ordinary keys, modifier keys, and shortcuts | They have an effect on the target and not on the source |
| AC-14 | Space swipes, Mission Control, Command+Tab while forwarding on macOS | They do not happen on the source and do not go to the target |
| AC-15 | Release shortcut | The session ends. Local input and the cursor come back |
| AC-16 | Escape 10 times in 5 seconds | Suppression and the session end, also when the main loop does not respond |
| AC-17 | System-reserved shortcuts, missing permission, Secure Event Input | Telescreen reports the limit. It does not claim capture |
| AC-18 | Disconnect, process crash, target hang, queue overflow | Both sides release all input state. Local control comes back |
| AC-19 | The portal display disappears from the source | Forwarding stops. The UI comes back. No input is suppressed in a moved window |
| AC-20 | Two sources try to control the same target device | The lease allows only one session |
| AC-21 | A peer with an unknown fingerprint dials, or is dialed | The connection is refused on both sides |
| AC-22 | Linux is the source and macOS is the target | AC-3 to AC-18 pass in this direction, with the Linux exits (pointer lock, shortcut inhibit) |
| AC-23 | The target selects "disconnect" or requests control back | The session ends on both sides. The reverse session can start |
| AC-24 | A generic USB device | The user can hand it off from the device list. It does not need to be a display |
| AC-25 | The installed macOS app connects on the LAN | The connection works (Local Network privacy) |

Performance: measure p50, p95, and p99 of capture to injection, event loss, and idle CPU/GPU. A network latency number must state how it was measured (round trip on one clock, or the clock synchronization method). There is no hard threshold yet.

## 10. Open questions

- Whether a pause should also switch the video input back automatically. Current decision: no.
- The lease arbitration and the recovery protocol (NET-6): message design and timeouts. Tentative proposal for v0.1.0: `docs/adr/0007-session-protocol.md`. The user decided to keep it tentative until the hardware tests (AC-18) run.
- Which special shortcuts to support beyond IN-5, and the per-platform list of shortcuts that cannot be captured.
- The effect of Wi-Fi delay spikes (spike 4: p99 about 85 ms) on the pointer in real use. Measure it in the product.
- Minimum OS versions for each package. macOS: 27.0 for v0.1.0, the only tested version (user, 2026-10-27). Linux: open.
