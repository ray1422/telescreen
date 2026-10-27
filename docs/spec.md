# Telescreen specification

Status: draft for v0.1.0, 2026-10-27. This file is the normative product specification. `docs/config.md` and `schema/config.schema.json` specify the config and the driver contract. `GLOSSARY.md` defines the terms. `docs/adr/` records why each decision was made. `docs/spikes.md` and `docs/hardware.md` record measured facts.

Requirement IDs (for example `FLOW-3`) are stable. Refer to them in tests, tasks, and reviews. "Must" is a requirement. "Should" is a strong recommendation. "Can" is a permission.

## 1. Purpose

Some computers share physical devices, for example one display that has video cables from more than one computer. Telescreen lets the user control these computers with one keyboard and one mouse. The user opens a portal on a computer, selects a different computer, and Telescreen switches the shared display to that computer. Then the keyboard and the mouse of the first computer control the second computer while the pointer is in the portal. The picture goes through the video cable. Telescreen does not send video.

Example setup: a MacBook with a built-in display and an external display. A Linux computer also connects to the external display. The keyboard and the mouse are on the Mac.

## 2. Scope

### 2.1 Versions

Versions follow Semantic Versioning. There is no stable release, so all versions are 0.x. In 0.x, a new feature increases the minor version, and a fix increases the patch version.

v0.1.0 and v0.2.0 are internal milestones. They get no tag and no release (user, 2026-10-27). The first release is v0.3.0.

| Version | Content |
| --- | --- |
| v0.1.0 | Display handoff, confirmation prompt with the hint, and keyboard and mouse forwarding, in both directions: macOS to Linux, and Linux to macOS. |
| v0.2.0 | Opt-in text clipboard sync. It syncs only in an active, authenticated session, with no loops between the peers, and it sends nothing after the user disables it or after a disconnect. |
| v0.3.0 | Hand back, FLOW-24, the verification model, and the operation drivers (section 10; user, 2026-10-27). Until v0.3.0, FLOW-6, FLOW-8, FLOW-10, FLOW-21, UI-5, and ADR 0002 apply as written. v0.3.0 changes them to the verification model. |
| v0.4.0 (candidate) | Separate clipboard stream: image clipboard (PNG) and texts larger than 256 KiB (CLIP-6). An image is sent only when the other peer pastes, with a "Pasting from <label>…" overlay. |

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
- **FLOW-21** From the handoff until the confirmation or the timeout of FLOW-11, the portal stays fullscreen and fixed in place, and it keeps the window focus. It cannot be switched away (user, 2026-10-27). On macOS the source suppresses the gestures and shortcuts of IN-10 and the clicks out of the portal in this time, and it activates the portal again if it loses the focus. The confirmation key and clicks in the portal still go to the portal. On Linux the portal inhibits the compositor shortcuts in this time. A Wayland client cannot take the focus back, so a click on a different output can still move the focus.
- **FLOW-11** If there is no confirmation in 60 seconds (fixed in v0.1.0; configurable in a later version, user, 2026-10-27), cancel the wait. The target removes the prompt. The UI shows the video input as unknown. Do not switch the video input back.
- **FLOW-12** If a handoff step fails, or `verify` is false: do not start forwarding, do not retry automatically, and do not claim a rollback. After a partial success, show the video input as unknown.

### 4.3 Forwarding and end

- **FLOW-13** While forwarding, the source captures, suppresses, and forwards its input to the target by two rules. Pointer motion, buttons, scroll, and gestures follow the cursor: they go to the target while the cursor is in the portal (FLOW-20). Keys follow the keyboard focus (FLOW-22). The source must not need window focus events from the target.
- **FLOW-14** Switch of the controlled computer: the cursor can leave the portal at each time (FLOW-20), and the user can click and type on the source. This is a seamless switch (user, 2026-10-27, ADR 0003). The session has no pause state: it stays in forwarding until it ends. Only the routes of single events change. The target display does not change, and no new confirmation is necessary.
- **FLOW-15** Portal edges with no adjacent local display stop the pointer.
- **FLOW-20** The cursor of the source is always free: the source does not freeze or lock it, and pointer motion is never suppressed (user, 2026-10-27). Only pointer input while the cursor is in the portal area goes to the target. Each key and button up goes where its down went (IN-3).
- **FLOW-22** The keyboard focus follows clicks, as on one computer with more displays (user, 2026-10-27). Keys go to the target while the portal window has the native keyboard focus of the source, also when the cursor is out of the portal. A click in the portal gives the portal the keyboard focus. The click also goes to the target (user, 2026-10-27). A click on a different window of the source takes the keyboard focus away from the portal by the normal OS rules. Moving the cursor alone does not change the keyboard focus.
- **FLOW-23** At each time, at most one input field on the two computers shows the keyboard focus (user, 2026-10-27: two blinking text cursors are incorrect behavior). When the portal loses the keyboard focus, the source sends `KeyboardAway`. The target then takes the keyboard focus from its applications with a focus holder that shows nothing (UI-7). When the portal gets the keyboard focus again, the source sends `KeyboardBack` before the click that caused it. The target removes the focus holder before it injects that click. At the session end, the target removes the focus holder.
- **FLOW-16** These events end the session: the release shortcut, the safety exit, a disconnect (heartbeat timeout or a closed connection), "Stop this session" on the target, "Control from here" on the target (FLOW-17), lease expiry, removal of the portal display, and revocation of a permission. Then the source stops suppression and restores the key state. The target releases all held keys and buttons. If the video input is not confirmed, show it as unknown. Do not claim an automatic recovery.
- **FLOW-17** The target shows "Controlled by <source label>" with two actions (user, 2026-10-27): "Stop this session" ends the session. "Control from here" ends the session, then starts a session in the other direction, with this computer as the source: a new connection that replaces the old one.
- **FLOW-18** A session end does not switch the video input back. A switch back needs an explicit handoff route. The user should be able to start the switch back from a display of the source that the handoff does not change (for example the built-in display).
- **FLOW-24** When a computer grants a session as the target, it closes its own portal on the input-target display of that session (user, 2026-10-27). Then the display does not show two portals, and the user knows which computer is the target. Portals on other displays stay open. If the closed portal is in a session as a source, that session ends as for each closed portal. There is no special rule for three or more computers, and nobody confirms this end (user, 2026-10-27). Example: A has a portal on display D, and it is the source of a session to C. The user switches D to B by hand, and requests a session to A from B's portal on D. A grants it and closes its portal on D, so the session A to C ends. There is no nested forwarding. The control window of A shows "Session ended: <B> took this display" (user, 2026-10-27).
  - With two computers, B is C. Then the request has the same result as "Control from here" (FLOW-17): the old session ends, and the new session starts.
- **FLOW-19** If the portal display disappears from the source, stop forwarding and restore the UI. Do not continue to suppress input in a window that moved to a different display.

### 4.4 Text clipboard (v0.2.0)

- **CLIP-1** Text clipboard sync is opt-in. The config key `clipboard.text` of each computer gives the start value (default `false`). The control window has a switch that changes it while the app runs (user, 2026-10-27). The switch is not saved.
- **CLIP-2** The sync works in both directions (user, 2026-10-27), only while a session is in forwarding, and only on the authenticated connection of that session. A peer sends its text only while its own switch is on. A peer applies a received text only while its own switch is on.
- **CLIP-3** A peer sends its clipboard text when the local clipboard changes during the session. It does not send the text that was on the clipboard before the session started (user, 2026-10-27). Reason: privacy. A text from before the session can be old, and the user did not copy it for this session.
- **CLIP-4** No loops: a peer does not send back a text that it set from the other peer.
- **CLIP-5** After the switch goes off, after the session ends, or after a disconnect, a peer sends nothing. It does not send a change from that time later.
- **CLIP-6** Only plain UTF-8 text. A text larger than 256 KiB is not sent, and the source shows a notice (user, 2026-10-27). Reason: the text uses the same reliable stream as the input events, and a large text delays the keys after it. A later milestone sends larger texts on a separate clipboard stream.
- **CLIP-7** A password manager can mark a clipboard item as concealed (macOS `org.nspasteboard.ConcealedType`; Linux `x-kde-passwordManagerHint` with the value `secret`). A peer sends a concealed item as other text, with the concealed mark (user, 2026-10-27). The receiving peer sets the same mark on its own clipboard, so that clipboard history tools do not record the item. When the concealed item goes off the clipboard of the sending peer (for example, the password manager clears it), the sending peer tells the other peer. The other peer then clears its clipboard if it still holds that item. All of this is best effort (user, 2026-10-27): each OS and each clipboard tool keeps its own limits, and Telescreen does not remove them. For example, some password managers on Linux do not set the mark; their items sync as other text. On Wayland, the clipboard also becomes empty when the program that copied quits; the sending peer cannot tell this from a clear, and does the same. Not verified: whether clipboard history tools on Linux other than KDE obey the mark, and whether macOS Universal Clipboard obeys it.
- **CLIP-9** No log and no debug output contains clipboard text (user, 2026-10-27).
- **CLIP-10** A paste on the receiving peer needs no extra authentication (user, 2026-10-27). Reason: the peers are paired (ADR 0004), and the user confirmed the session at the prompt. Also, a pushed text is already on the local clipboard, so Telescreen cannot see the paste.
- **CLIP-11** When the session ends, for each cause, a peer clears its clipboard if it still holds a text that it set from the other peer (user, 2026-10-27). This includes texts with no concealed mark. Reason: the connection ends, and the text must not stay. The peer does the same when its switch goes off (Claude decides, delegated by the user, 2026-10-27). This is local: the peer sends nothing (CLIP-5). If the app stops unexpectedly, the peer cannot clear. On Wayland, the text also goes off the clipboard when Telescreen quits, because Telescreen provides it.
- **CLIP-12** Only the clipboard selection syncs. The Linux primary selection (select and middle-click paste) does not sync (Claude decides, delegated by the user, 2026-10-27). Reason: macOS has no primary selection.
- **CLIP-8** The clipboard is a platform capability behind an interface. macOS uses `NSPasteboard`. Linux uses the data-control protocol on its own Wayland connection. v0.2.0 uses `zwlr-data-control-unstable-v1`; `ext-data-control-v1` can come later. If the compositor does not have it (for example GNOME), the capability is missing. Then the control window does not show the switch, and the app shows a notice with the cause at start (Claude decides, delegated by the user, 2026-10-27). The input callbacks never read the clipboard (IN-2).

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
- **IN-10** While forwarding on macOS, suppress all gestures and shortcuts that switch Spaces or applications, or that open Mission Control, by the rules of FLOW-13: shortcuts while the portal has the keyboard focus, and gestures while the cursor is in the portal. Do not forward gestures (ADR 0003). At other times, they go to macOS.
- **IN-11** Pointer position: the source maps the pointer position in the portal, normalized to the portal size, to the input-target display of the target. The backends handle scale, rotation, negative origins, and pixel coordinates. The pointer must not leave the input-target display on the target.
- **IN-12** If the OS blocks capture (for example macOS Secure Event Input, or a missing permission), report it clearly. Do not claim that capture works.
- **IN-13** Input lost: if the capture can no longer see the up events of the forwarded keys, the source releases all held state of the session and sends `ReleaseAll`. The session continues. Examples: the Linux portal loses the keyboard focus (other windows get the up events), macOS turns on Secure Event Input during the session, and macOS disables the event tap for a time.

## 6. Portal window and prompt

- **UI-1** The portal is borderless and covers its display. On macOS it is a native fullscreen Space, above the menu bar level (window level 26), so the menu bar and the title bar do not show at the top edge (ADR 0003, spike 1).
- **UI-2** While the portal has the focus, it allows only Roman input sources, and macOS restores the previous input source after it (spike 1).
- **UI-3** If the portal leaves fullscreen (for example a drag in Mission Control), detect it. Then the source starts a hand back (user, 2026-10-27; section 10). There is no pause state (FLOW-14). v0.1.0 and v0.2.0 do not detect it (user, 2026-10-27): the session continues.
- **UI-4** The portal shows the candidate targets, the state of the session (waiting for confirmation, forwarding, ended), and the video input state (confirmed, unverified, unknown).
- **UI-5** The prompt on the target is an overlay on the input-target display. On Linux use layer-shell where the compositor has it; else a normal fullscreen window.
- **UI-6** On Linux as the source, the portal is a fullscreen window. While forwarding, it inhibits compositor shortcuts (ADR 0006). It does not lock the pointer (FLOW-20). The keyboard focus is the native focus of the portal window (FLOW-22). When the portal loses it, the source sends `KeyboardAway` and applies IN-13. The shortcut inhibit is decided (user, 2026-10-27). When the portal opens, it requests the inhibit one time, so that a compositor that asks the user for permission asks then, and not after the handoff. Then it releases the inhibit, and it requests it again for the confirmation wait (FLOW-21) and for forwarding. If the compositor refuses, the portal shows a notice when it opens. niri asks for no permission. Not verified: whether GNOME keeps the permission after the release.
- **UI-7** The focus holder on the target (FLOW-23) is a platform capability behind an interface, so that more backends can come later. It shows nothing and does not take pointer input. On Linux it is a layer-shell surface of one pixel with exclusive keyboard interactivity and an empty input region, on the input-target output. On macOS it is a transparent Telescreen window that becomes the key window; when it is removed, the application that was frontmost before is activated again. If the target has no focus holder backend (for example GNOME, which has no layer-shell), the target accepts the session (user, 2026-10-27). It sends one desktop notification at the session start: this computer cannot hide its text cursor, so the cursor can still blink here when the source has the keyboard focus, but the keys do not come here. The row of the session in the control window shows the same warning while the session runs. Later losses of the keyboard focus give no notification. On GNOME, an app cannot place a window, keep it on top, or make an overlay; only a GNOME Shell extension can. GNOME support is a later goal (user, 2026-10-27); see section 10.

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
| Capture and suppress | CGEventTap at the HID location (Accessibility, Input Monitoring) | Window input of the portal + keyboard shortcuts inhibit |
| Pointer injection | `CGEventPost` | wlroots virtual pointer, absolute motion on the input-target output |
| Key injection | `CGEventPost` | uinput keyboard (the compositor keymap applies) |
| Prompt overlay | AppKit window above the fullscreen level | layer-shell overlay |
| Focus holder (UI-7) | transparent key window | layer-shell surface, exclusive keyboard |
| Text clipboard (CLIP-8) | `NSPasteboard` | data-control protocol |
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
| AC-6 | Press the confirmation key with no prompt | It is a normal key. Forwarding does not start |
| AC-7 | No confirmation in the timeout | The wait stops. The prompt goes away. The video input shows as unknown. It does not switch back |
| AC-8 | The portal moves to a different display | `portal-match` runs again. The old binding and coordinates do not apply |
| AC-9 | `portal-match` false / more than one true / timeout | No match / ambiguous / error. No device is selected arbitrarily |
| AC-10 | A handoff step fails, or `verify` is false | Forwarding does not start. No retry. No rollback claim |
| AC-11 | The pointer leaves the portal to the built-in display, then comes back, with no click | Pointer input is local while out; keys still go to the target; the session stays in forwarding. No key or button stays pressed on the target |
| AC-12 | Resolution, scale, rotation, negative coordinates, different aspect ratios | The target pointer position is correct. Out-of-bounds behavior is defined |
| AC-13 | Ordinary keys, modifier keys, and shortcuts | They have an effect on the target and not on the source |
| AC-14 | Space swipes, Mission Control, Command+Tab while forwarding on macOS | They do not happen on the source and do not go to the target |
| AC-15 | Release shortcut | The session ends. Local input comes back |
| AC-16 | Escape 10 times in 5 seconds | Suppression and the session end, also when the main loop does not respond |
| AC-17 | System-reserved shortcuts, missing permission, Secure Event Input | Telescreen reports the limit. It does not claim capture |
| AC-18 | Disconnect, process crash, target hang, queue overflow | Both sides release all input state. Local control comes back |
| AC-19 | The portal display disappears from the source | Forwarding stops. The UI comes back. No input is suppressed in a moved window |
| AC-20 | Two sources try to control the same target device | The lease allows only one session |
| AC-21 | A peer with an unknown fingerprint dials, or is dialed | The connection is refused on both sides |
| AC-22 | Linux is the source and macOS is the target | AC-3 to AC-18 pass in this direction, with the Linux exits (shortcut inhibit) |
| AC-23 | The target selects "Stop this session" or "Control from here" | The session ends on both sides. With "Control from here", the session in the other direction starts |
| AC-24 | A generic USB device | The user can hand it off from the device list. It does not need to be a display |
| AC-25 | The installed macOS app connects on the LAN | The connection works (Local Network privacy) |
| AC-26 | While the source waits for the confirmation, try to switch the Space or the application, or click a different display | The portal stays fullscreen and keeps the focus (FLOW-21) |
| AC-27 | Type in a field on the target, click a field on the source, type, then click the field on the target again | After each click, only the clicked field shows the keyboard focus, and the keys go there (FLOW-22, FLOW-23). The click in the portal reaches the target |
| AC-28 | Hold Shift in the portal, then lose the keyboard focus without a key up (Linux: a compositor shortcut that cannot be inhibited) | The target releases Shift (IN-13). The session continues |
| AC-29 | A target with no focus holder backend | The session starts. One desktop notification and a warning in the control window (UI-7) |
| AC-30 | Switch on at both peers. Copy text on the source, then paste on the target; then copy on the target and paste on the source | The text arrives in both directions (CLIP-2) |
| AC-31 | Copy text before the session starts | The text is not sent (CLIP-3) |
| AC-32 | Switch off on one peer, or end the session, then copy | Nothing is sent (CLIP-2, CLIP-5) |
| AC-33 | Copy on one peer | The other peer does not send the same text back (CLIP-4) |
| AC-34 | Copy a password from a password manager, then wait until the password manager clears its clipboard | The password arrives with the concealed mark; then the other peer clears it (CLIP-7) |
| AC-35 | Copy text on one peer. End the session, or turn the switch off on the other peer | The other peer clears its clipboard. If the user copied something else there before, it stays (CLIP-11) |

Performance: measure p50, p95, and p99 of capture to injection, event loss, and idle CPU/GPU. A network latency number must state how it was measured (round trip on one clock, or the clock synchronization method). There is no hard threshold yet.

## 10. Open questions

- GNOME as the target (user, 2026-10-27: GNOME must be supported in the end). GNOME has no layer-shell, so the prompt (UI-5) and the focus holder (UI-7) need a different backend behind the same interfaces. A candidate is a GNOME Shell extension. X11 is not an option (user, 2026-10-27).

- The lease arbitration and the recovery protocol (NET-6): message design and timeouts. Tentative proposal for v0.1.0: `docs/adr/0007-session-protocol.md`. The user decided to keep it tentative until the hardware tests (AC-18) run.
- Which special shortcuts to support beyond IN-5, and the per-platform list of shortcuts that cannot be captured.
- The effect of Wi-Fi delay spikes (spike 4: p99 about 85 ms) on the pointer in real use. Measure it in the product.
- Minimum OS versions for each package. macOS: 27.0 for v0.1.0, the only tested version (user, 2026-10-27). Linux: open.
- Hand back (user, 2026-10-27). These points are decided:
  - The name is "Hand back". It reverses each device that the session handed off.
  - The target runs its own `handoff.<source>` route. If the target has no such route, the action is not available. The button stays in its place, is disabled, and shows the cause, for example "No route back to <source> in this config" (user, 2026-10-27).
  - After the switch, the source runs its own `verify` of the device (see the verification model below).
  - The target row shows three actions: "Hand back" (the main action), "Control from here", and "Disconnect". "Disconnect" is the new label of "Stop this session". The end reason stays `TargetStopped`.
  - The action is only on the target. In normal use, no window of the source is visible while the display shows the target. FLOW-18 must change: its last sentence assumes such a window.
- Operation drivers (user, 2026-10-27; to apply to `docs/config.md`, the schema, the validator, and the contract tests):
  - A driver defines how Telescreen reads the result of a program. It does not define which program runs. The user writes the whole command, also the program.
  - `command` is a string or a list, as in Docker Compose. A string is split by shell rules, but no shell runs it. A list is used as it is. For a pipe or other shell features, write the shell explicitly, for example `[/bin/sh, -c, "a | b"]`.
  - Steps run in order and stop at the first error (as now).
  - Drivers: `exec` (exit code 0 is success or true), `command` (protocol 1, JSON), and drivers for the output format of one tool, for example `betterDisplay` (JSON on stdout) and `ddcutil` (text). A tool driver can compare a value with `expect`.
  - `open` on macOS starts an app through Launch Services and returns at once. The output of the app does not come back. Use the executable in the app bundle, or `open --stdout`.
  - A program name with no path is found in the search path (user, 2026-10-27). If the computer section of the config has `path` (a list of directories), use it. Else read `PATH` from the login shell of the user one time at the start. An app that macOS starts from Finder or `open` has only the system directories in its own `PATH`.
  - No `verify` means no check (user, 2026-10-27). Then forwarding starts when the steps succeed, and the video input shows "unverified". The example configs must add an explicit prompt to keep the current behavior.
- Verification model (user, 2026-10-27; it replaces ADR 0002 in part and needs a new ADR):
  - `verify` is in the device section of the destination computer, the computer that the device switches to. The destination runs it. The route of the computer that runs the steps has no `verify`.
  - A verifier talks with the core in JSON lines. It can show text on the switched display (`{"show":…}`) and ask for an acknowledgement of the user (`{"want":"ack"}`). The core replies `{"ack":true}` when the user acknowledges. The purpose is to make sure that a user sees the switched display (user, 2026-10-27). The verifier does not know how the user acknowledged. It replies with ok or not ok.
  - The core decides which user actions are an acknowledgement (user, 2026-10-27). In the portal of the computer that started the handoff (through the network), if that computer has a portal: Return, or a click in the portal. Else, on the destination: Return, or a click on the prompt window. A hand back always uses the actions on the destination, also when the source requested it (user, 2026-10-27): the session already ended, and the user then uses the keyboard of the destination.
  - The protocol has no other user input. A verifier that needs other input (for example the state of a Bluetooth keyboard) reads it itself on the destination (user, 2026-10-27).
  - The built-in `prompt` verifier is one verifier with this protocol. It shows the prompt with the hint, asks for an acknowledgement, and replies ok. After 60 seconds with no acknowledgement, it replies not ok.
  - A failed or timed-out verification makes the state of the device "unknown" on the destination. The device list shows "It is back" for that device. The user can press it to set the state to confirmed. This state does not depend on a session.
- Hand back (user, 2026-10-27) is not a special flow. It is three steps: the session ends as for "Disconnect"; the target runs its own `handoff.<source>` route as a handoff with no portal and no forwarding; then the source runs its own `verify` of the device.
- Hand back scope: one device (the device of the session). The device state model is per device, so device groups can come later.
- A window that does not depend on the portal (a hub) is a different topic.
- The source can also start a hand back (user, 2026-10-27). It sends a request, and the target runs its own `handoff.<source>` route. This changes the rule of `docs/config.md` that no computer gives the execution to another computer. Only the peer of an active, authenticated session can request it, and it can only cause the route back to itself. Record this in a new ADR.
  - Triggers on the source: the portal leaves fullscreen (UI-3), and the shortcut that was the release shortcut (IN-7). Its new name is the hand back shortcut. The safety exit (IN-8) does not change: it only ends the session.
  - If the target has no route to the source, the session ends, and the target shows a notice that the user must switch the display by hand. The target shows it, because the display shows the target. The source also adds the notice to its control window.
  - The built-in `prompt` verifier has its own window on the switched display. It does not use the portal, because the portal can be gone.

Decisions of the implementation (Claude, 2026-10-27). The user confirmed the items that name the user:

- The texts of the session end on the source (user, 2026-10-27), with the target label:
  - `TargetStopped` ("Disconnect" on the target): "<target> disconnected the session".
  - `TargetDisconnected` (the target app quit and told the source): "Telescreen quit on <target>".
  - `PeerLost` (heartbeat timeout or a closed connection): "Connection to <target> lost".
- "Control from here" (FLOW-17) needs a device match for the display (user, 2026-10-27). If no device matches, the target shows a notice, does not start a handoff, and the old session continues.
- CLIP-8: v0.2.0 uses only `zwlr-data-control-unstable-v1`. `ext-data-control-v1` needs a staging feature of `wayland-protocols` and can come later.
- The Linux clipboard backend reads an offer for at most 1 second. A text larger than 256 KiB is cut to its longest valid UTF-8 prefix, so that the core rejects it as too large (CLIP-6).
- The controller does not read the clipboard while the switch is off or while no session can sync (CLIP-2). On Linux, the data-control connection is open only while the switch is on and a session is in forwarding (user, 2026-10-27). At start, the app only checks that the compositor has data-control (CLIP-8). When the connection opens, the current selection is the old text of CLIP-3. When the connection closes, Wayland removes a text that Telescreen still provides (CLIP-11).
