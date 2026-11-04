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
| v0.2.0 | Text clipboard sync. It syncs only in an active, authenticated session, with no loops between the peers, and it sends nothing after the user disables it or after a disconnect. |
| v0.3.0 | Hand back, FLOW-24, the verification model, and the operation drivers (section 10; user, 2026-10-27). |
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
- **PEER-3** The user selects the local computer ID. Telescreen must not guess it from the host name, also when the config has one computer. If no ID is saved, the app asks one time in the start window (UI-8), and saves the answer in the local data directory (user, 2026-11-04). The answer is not in the config, because all computers can share one config. The `--computer` argument has priority over the saved ID and is not saved. If the saved ID is not in the config, the app asks again. `telescreen forget-computer` removes the saved ID.
- **PEER-4** Each peer reads the same config format. Each peer runs only the operations of its own computer (`docs/config.md`).
- **PEER-5** `telescreen` with no argument is the same as `telescreen run`, so that a launcher can start it (user, 2026-11-04). The default config is `$XDG_CONFIG_HOME/telescreen/config.yaml`, else `~/.config/telescreen/config.yaml`, on all platforms. The `--config` argument replaces it. Telescreen does not write the config.

## 4. User flow

### 4.1 Portal and candidates

- **FLOW-1** Read and validate the config. Detect the local capabilities (input capture, injection, permissions).
- **FLOW-2** The user opens a portal. The portal is not in the config. The control window lists the displays, and each display has its own "Open portal" button (user, 2026-11-05; this replaces the rule that the portal opens on the display of the control window). The control window can be on a different display. If a portal is open on a different display, the new portal replaces it, and the session of the old portal ends. On the display of the portal, evaluate `portal-match` of the local devices. No match: tell the user. More than one match: the user selects one. Do not use the first match silently.
- **FLOW-3** Make the candidate targets: the local `handoff` keys that are also computers with the same device `id`. Show a candidate that is offline, that has no permission, or that has no `input-target` as unavailable, with the reason.
- **FLOW-4** When the portal moves to a different display, or the display topology changes, evaluate `portal-match` again. The previous device binding and its coordinates must not continue to apply.

### 4.2 Handoff and verification

- **FLOW-5** After the user selects a target, the target resolves its local `input-target` by the shared device `id`, confirms that it can inject input, and gives an exclusive lease for that device to this session.
- **FLOW-6** The source stops any old input stream and releases all held keys and buttons. Then it runs the handoff `steps` on the source computer. When the steps succeed, the target verifies the handoff as the destination (VER-1).
- **FLOW-7** With the `prompt` verifier, the target shows the prompt on its input-target display, with the `hint` of the target computer as plain text (VER-5).
- **FLOW-8** When the verifier of the target asks for an acknowledgement, the user acknowledges in the portal: the acknowledgement key (Return. In v0.3.0 the key is fixed; a later version makes it configurable, user, 2026-10-27), or a click in the portal (VER-4). The source accepts it only while the target reports that its verifier waits for an acknowledgement, and the portal has the window focus. At other times the key is a normal key.
- **FLOW-9** The source suppresses the acknowledgement key and the acknowledgement click. It does not forward them. The click on the switch button must not go to the target.
- **FLOW-10** Forwarding starts only after the verifier of the target answers ok, or after the steps if the target has no `verify` (VER-2), and only if the display token of the target is still valid. The UI shows the device state that the target reports (VER-6). If the input-target display disappears while the session forwards, the target ends the session with `DisplayGone`: it releases all held input and does not run a route back (a hand back runs only on a request, HB-1). After the display is back, the user starts a new handoff (2026-10-28).
- **FLOW-21** From the handoff until the verification ends (FLOW-10, FLOW-11), the portal stays fullscreen and fixed in place, and it keeps the window focus. It cannot be switched away (user, 2026-10-27). On macOS the source suppresses the gestures and shortcuts of IN-10 and the clicks out of the portal in this time, and it activates the portal again if it loses the focus. The acknowledgement key and clicks in the portal still go to the portal. On Linux the portal inhibits the compositor shortcuts in this time. A Wayland client cannot take the focus back, so a click on a different output can still move the focus.
- **FLOW-11** If the verifier answers not ok (for example the `prompt` verifier after 60 seconds with no acknowledgement, VER-5), or the `verify` operation fails, cancel the wait. The target removes the window of its verifier. The device state is unknown (VER-6). Do not switch the video input back.
- **FLOW-12** If a handoff step fails: do not start forwarding, do not retry automatically, and do not claim a rollback. After a partial success, the device state is unknown.

### 4.3 Forwarding and end

- **FLOW-13** While forwarding, the source captures, suppresses, and forwards its input to the target by two rules. Pointer motion, buttons, scroll, and gestures follow the cursor: they go to the target while the cursor is in the portal (FLOW-20). Keys follow the keyboard focus (FLOW-22). The source must not need window focus events from the target.
- **FLOW-14** Switch of the controlled computer: the cursor can leave the portal at each time (FLOW-20), and the user can click and type on the source. This is a seamless switch (user, 2026-10-27, ADR 0003). The session has no pause state: it stays in forwarding until it ends. Only the routes of single events change. The target display does not change, and no new acknowledgement is necessary.
- **FLOW-15** Portal edges with no adjacent local display stop the pointer.
- **FLOW-20** The cursor of the source is always free: the source does not freeze or lock it, and pointer motion is never suppressed (user, 2026-10-27). Only pointer input while the cursor is in the portal area goes to the target. Each key and button up goes where its down went (IN-3).
- **FLOW-22** The keyboard focus follows clicks, as on one computer with more displays (user, 2026-10-27). Keys go to the target while the portal window has the native keyboard focus of the source, also when the cursor is out of the portal. A click in the portal gives the portal the keyboard focus. The click also goes to the target (user, 2026-10-27). A click on a different window of the source takes the keyboard focus away from the portal by the normal OS rules. Moving the cursor alone does not change the keyboard focus.
- **FLOW-23** At each time, at most one input field on the two computers shows the keyboard focus (user, 2026-10-27: two blinking text cursors are incorrect behavior). When the portal loses the keyboard focus, the source sends `KeyboardAway`. The target then takes the keyboard focus from its applications with a focus holder that shows nothing (UI-7). When the portal gets the keyboard focus again, the source sends `KeyboardBack` before the click that caused it. The target removes the focus holder before it injects that click. At the session end, the target removes the focus holder.
- **FLOW-16** These events end the session: a hand back (HB-1), the safety exit, a disconnect (heartbeat timeout or a closed connection), "Disconnect" on the target, "Control from here" on the target (FLOW-17), lease expiry, removal of the portal display, and revocation of a permission. Then the source stops suppression and restores the key state. The target releases all held keys and buttons. If the device state is not confirmed, show it as unknown. Do not claim an automatic recovery.
- **FLOW-17** The target shows "Controlled by <source label>" with three actions (user, 2026-10-27): "Hand back" (the main action, HB-2), "Control from here", and "Disconnect". "Disconnect" ends the session with no switch. "Control from here" ends the session, then starts a session in the other direction, with this computer as the source: a new connection that replaces the old one.
- **FLOW-18** Only a hand back (section 4.6) switches the device back. The other ends of FLOW-16 do not switch the video input. A different switch needs an explicit handoff route.
- **FLOW-24** When a computer grants a session as the target, it closes its own portal on the input-target display of that session (user, 2026-10-27). Then the display does not show two portals, and the user knows which computer is the target. Portals on other displays stay open. If the closed portal is in a session as a source, that session ends as for each closed portal. There is no special rule for three or more computers, and nobody confirms this end (user, 2026-10-27). Example: A has a portal on display D, and it is the source of a session to C. The user switches D to B by hand, and requests a session to A from B's portal on D. A grants it and closes its portal on D, so the session A to C ends. There is no nested forwarding. The control window of A shows "Session ended: <B> took this display" (user, 2026-10-27).
  - With two computers, B is C. Then the request has the same result as "Control from here" (FLOW-17): the old session ends, and the new session starts.
- **FLOW-19** If the portal display disappears from the source, stop forwarding and restore the UI. Do not continue to suppress input in a window that moved to a different display.

### 4.4 Text clipboard (v0.2.0)

- **CLIP-1** Text clipboard sync is on by default (user, 2026-10-27). Reason: the sync works only in a session that the user started, and Telescreen does not keep the texts. The config key `clipboard.text` of each computer gives the start value (default `true`). The control window has a switch that changes it while the app runs (user, 2026-10-27). The switch is not saved.
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
- **CLIP-8** The clipboard is a platform capability behind an interface. macOS uses `NSPasteboard`. Linux uses the data-control protocol on its own Wayland connection. Linux uses `ext-data-control-v1` (user, 2026-11-04). KWin has only this protocol, and niri and Hyprland also have it. If the compositor does not have it (for example GNOME), the capability is missing. Then the control window does not show the switch, and it shows a notice with the cause from the start (Claude decides, delegated by the user, 2026-10-27). The input callbacks never read the clipboard (IN-2).

### 4.5 Verification (v0.3.0)

ADR 0009. The **destination** is the computer that a handoff switches the device to.

- **VER-1** `verify` is in the device section of the destination. The destination runs it after the steps of each handoff to it, also of a hand back. A handoff route has no `verify`. The computer that runs the steps tells the destination over the network when the steps succeed.
- **VER-2** No `verify` means no check (user, 2026-10-27). Forwarding starts when the steps succeed, and the device state is unverified.
- **VER-3** A verifier talks with the core in JSON lines (`docs/config.md`). It can show text on the switched display (`{"show":…}`) and ask for an acknowledgement of the user (`{"want":"ack"}`). The core answers `{"ack":true}` when the user acknowledges. The verifier answers ok or not ok. It does not know how the user acknowledged. The purpose is to make sure that a user sees the switched display (user, 2026-10-27).
- **VER-4** The core decides which user actions are an acknowledgement (user, 2026-10-27). If the computer that started the handoff has a portal: Return, or a click in that portal, sent over the network (FLOW-8). Else, on the destination: Return in a window of Telescreen on the destination, or a click in the window of the verifier or in the portal (user, 2026-10-27). A click in the control window is not an acknowledgement, because it can hit a button (user, 2026-10-27). A hand back always uses the actions on the destination, also when the source requested it: the session already ended, and the user then uses the keyboard of the destination.
- **VER-5** The built-in `prompt` verifier shows the prompt with the `hint` of the destination in its own window on the switched display (UI-5), asks for an acknowledgement, and answers ok. After 60 seconds with no acknowledgement (fixed in v0.3.0), it answers not ok. It does not use the portal, because the portal can be gone.
- **VER-6** The device state is per device on the destination, and it does not depend on a session: confirmed (the verifier answered ok), unverified (no `verify`), or unknown (a failed or timed-out verification, or a partial handoff). After a hand back, the computer that ran its route back changes its own state: a success sets "no state" (the device is not on this computer), and a partial failure sets unknown. A failure with no completed step does not change the state (user, 2026-10-27). For an unknown device, the device list shows "It is back". The user presses it to set the state to confirmed. The source of a session shows the state that the target reports (UI-4).
- **VER-7** The protocol has no other user input. A verifier that needs other input (for example the state of a Bluetooth keyboard) reads it itself on the destination (user, 2026-10-27).

### 4.6 Hand back (v0.3.0)

- **HB-1** A hand back switches the device of a session back to the source (user, 2026-10-27). It is not a special flow, but three steps: the session ends as for "Disconnect"; the target runs its own `handoff.<source>` route for the device, as a handoff with no portal and no forwarding; then the source verifies as the destination (VER-1). It covers one device: the device of the session. FLOW-12 applies to the steps.
- **HB-2** On the target, each incoming session has "Hand back" as the main action (FLOW-17), in the control window and in the menu of the status icon (UI-4). If the target has no `handoff.<source>` route for the device, the action stays in its place, is disabled, and shows the cause, for example "No route back to <source> in this config" (user, 2026-10-27).
- **HB-3** The source can also start a hand back (ADR 0010): it sends a request, and the target runs its own `handoff.<source>` route. Only the source of an active, authenticated session can request it, and it causes only the route back to the source. The triggers on the source are the hand back shortcut (IN-7) and a portal that leaves fullscreen (UI-3). The source has no "Hand back" button: in normal use, no window of the source is visible while the display shows the target.
- **HB-4** If the target has no route back when the source requests a hand back, the session ends, and the target shows a notice that the user must switch the display by hand. The target shows it, because the display shows the target. The source adds the same notice to its control window.
- **HB-5** The safety exit (IN-8) only ends the session. It does not start a hand back.

## 5. Input

- **IN-1** Capture, suppress, forward, and inject are four separate capabilities. Detect and report each one. Window key events and global hotkey registration are not input capture.
- **IN-2** Input callbacks (event taps, Wayland input events) only capture, decide to suppress, and push to a bounded queue. They must not run helpers, wait for the network, or block the UI.
- **IN-3** Key and button down/up events must stay in pairs. Pointer motion can be merged. Key and button events must not be dropped. If the queue overflows, end the session and release all held state.
- **IN-4** Both peers release all held keys and buttons on a session end. The target must release them on a heartbeat timeout by itself; it must not wait for key-up events from the source.
- **IN-5** Keys go by physical key identity, not by the character. The input method of the source must not change the key (spike 1). Modifier keys go by physical key: Command to Super, Option to Alt, Control to Control.
- **IN-6** Do not remap shortcuts. The `hint` tells the user the shortcuts of the target.
- **IN-7** The hand back shortcut (macOS: Control+Option+Command+Escape. Linux: Control+Alt+Shift+Escape, user, 2026-10-27. Fixed in v0.3.0; configurable in a later version) starts a hand back (HB-3). It is never forwarded.
- **IN-8** The safety exit (Escape 10 times in 5 seconds) ends suppression and the session, with no hand back (HB-5). The input callback counts the presses itself, with no dependency on the main thread, the UI, or the network. Escape is still forwarded.
- **IN-9** A watchdog that is independent of input events must stop suppression when the main loop does not respond for a few seconds, and when a suppression deadline passes (spike 2).
- **IN-10** While forwarding on macOS, suppress all gestures and shortcuts that switch Spaces or applications, or that open Mission Control, by the rules of FLOW-13: shortcuts while the portal has the keyboard focus, and gestures while the cursor is in the portal. Do not forward gestures (ADR 0003). At other times, they go to macOS.
- **IN-11** Pointer position: the source maps the pointer position in the portal, normalized to the portal size, to the input-target display of the target. The backends handle scale, rotation, negative origins, and pixel coordinates. The pointer must not leave the input-target display on the target.
- **IN-14** The target multiplies each forwarded scroll by the `scroll-speed` of its own computer section (`docs/config.md`). The default is 1. The value applies only to forwarded scrolls: local input of the target does not change. Reason: the correct speed depends on the applications and on the compositor of the target (user, 2026-11-04). A target that can inject only whole pixels keeps the remainder for the next scroll, so that a slow scroll does not get lost.
- **IN-15** On Linux, the pointer injection does not need the wlroots virtual pointer (user, 2026-11-04). If the compositor has `zwlr_virtual_pointer_manager_v1` (niri), the injector uses it. Else the injector makes a uinput absolute pointer (KWin). The compositor maps the range of that device to all outputs together, so the injector computes the position from the logical bounds of the outputs, for each motion. Its scroll has two forms (user, 2026-11-05; spike 5):
  - On KWin, unless the quirk `kwin-button-scroll` is off (IN-16), the injector uses a second uinput device with button scroll, because applications animate a wheel scroll and KWin sends a different scroll source only from a libinput device. The source of this scroll is `continuous`. The middle button of this device stays down from the start of a session. No client sees this button. The injector presses it again at the start of each session. It lets the button go up only after a motion of the device, so that libinput sends no click. The injector sets the KWin scroll factor of this device (KWin stores it in `kcminputrc`). KWin shows a scroll cursor for a device with button scroll on. Thus while the button is down, the injector turns off the KWin setting for button scroll: libinput applies this change only after the button goes up, so the scroll continues and the cursor does not change. Before each button down, the injector turns on the setting and reads it back. If KWin does not confirm it, the injector does not press the button and uses the wheel scroll.
  - On each other compositor, and if the quirk is off, the scroll is a wheel scroll with high-resolution values. Reason: the button scroll was tested only on KWin.
  - On both forms, 60 logical pixels of a scroll are one wheel notch. Thus `scroll-speed` (IN-14) has the same effect on both forms.
  - Limit: if a VT change or a compositor restart occurs during a session, libinput can lose the button state. Then a scroll can move the pointer until the next session starts.
- **IN-16** A quirk is a platform workaround with a name (user, 2026-11-05). Each quirk is on by default. The `disabled-quirks` list in the computer section (`docs/config.md`) turns off a quirk on that computer. Thus the user can turn off a workaround that fails after an update of the platform, with no new build. The quirk of IN-15 is `kwin-button-scroll`.
- **IN-12** If the OS blocks capture (for example macOS Secure Event Input, or a missing permission), report it clearly. Do not claim that capture works.
- **IN-13** Input lost: if the capture can no longer see the up events of the forwarded keys, the source releases all held state of the session and sends `ReleaseAll`. The session continues. Examples: the Linux portal loses the keyboard focus (other windows get the up events), macOS turns on Secure Event Input during the session, and macOS disables the event tap for a time.

## 6. Portal window and prompt

- **UI-1** The portal is borderless and covers its display. On macOS it is a native fullscreen Space, above the menu bar level (window level 26), so the menu bar and the title bar do not show at the top edge (ADR 0003, spike 1).
- **UI-2** While the portal has the focus, it allows only Roman input sources, and macOS restores the previous input source after it (spike 1).
- **UI-3** If the portal leaves fullscreen (for example a drag in Mission Control), detect it. Then the source requests a hand back (HB-3). There is no pause state (FLOW-14).
- **UI-4** The portal shows the candidate targets, the state of the session (waiting for the verification, forwarding, ended), and the device state (confirmed, unverified, unknown; VER-6).
  - From v0.3.0, the state also shows where the user can see it while the display shows the target (user, 2026-10-27). With one display, the source is then not visible, so the target must show it:
    - The control window of each peer shows the session state and the device state: the source in a "current session" part, the target in each row of the incoming sessions.
    - Each peer has a status icon: on macOS in the menu bar (`NSStatusItem`), on Linux as a StatusNotifierItem over D-Bus (not the X11 tray). The target icon shows who controls it and the state, for example "Controlled by <source> · forwarding". The source icon shows which target it controls.
    - The icon menu opens the control window. On the target, it also has the actions of each incoming session: "Hand back", "Control from here", and "Disconnect" (Claude decides, delegated by the user, 2026-10-27).
    - A StatusNotifierItem is visible only with a host (KDE, or a bar such as waybar; GNOME needs the AppIndicator extension). Without a host, only the control window shows the state.
- **UI-5** The `prompt` verifier (VER-5) shows its window on the switched display of the destination. On macOS it is a window above the fullscreen level. On Linux it is a layer-shell overlay where the compositor has it; else a normal fullscreen window.
- **UI-6** On Linux as the source, the portal is a fullscreen window. While forwarding, it inhibits compositor shortcuts (ADR 0006). It does not lock the pointer (FLOW-20). The keyboard focus is the native focus of the portal window (FLOW-22). When the portal loses it, the source sends `KeyboardAway` and applies IN-13. The shortcut inhibit is decided (user, 2026-10-27). When the portal opens, it requests the inhibit one time, so that a compositor that asks the user for permission asks then, and not after the handoff. Then it releases the inhibit, and it requests it again for the verification wait (FLOW-21) and for forwarding. If the compositor refuses, the portal shows a notice when it opens. niri asks for no permission. Not verified: whether GNOME keeps the permission after the release.
- **UI-7** The focus holder on the target (FLOW-23) is a platform capability behind an interface, so that more backends can come later. It shows nothing and does not take pointer input. On Linux it is a layer-shell surface of one pixel with exclusive keyboard interactivity and an empty input region, on the input-target output. On macOS it is a transparent Telescreen window that becomes the key window; when it is removed, the application that was frontmost before is activated again. If the target has no focus holder backend (for example GNOME, which has no layer-shell), the target accepts the session (user, 2026-10-27). It sends one desktop notification at the session start: this computer cannot hide its text cursor, so the cursor can still blink here when the source has the keyboard focus, but the keys do not come here. The row of the session in the control window shows the same warning while the session runs. Later losses of the keyboard focus give no notification. On GNOME, an app cannot place a window, keep it on top, or make an overlay; only a GNOME Shell extension can. GNOME support is a later goal (user, 2026-10-27); see section 10.
- **UI-8** A launcher shows no error output, so the start window shows what stops the start (user, 2026-11-04). It opens before the network, the capture, and the permission requests. It lists the computers of the config, and the user selects this computer. If the config is missing or not valid, the window shows the path and the problem, and has the button "Check again". If the app started with no argument and the network or a platform part cannot start, the window shows that problem. With an explicit `--config` or `--computer`, a problem goes to stderr and the exit code is not 0, as for each command.

## 7. Network and security

- **NET-1** The transport is an abstraction with a reliable ordered channel (control messages, key and button events) and an unreliable channel for the latest pointer position (ADR 0001). Key and button events must use the reliable channel.
- **NET-2** v0.1.0 uses QUIC: reliable streams for the reliable channel, datagrams for the pointer. Each pointer datagram has a sequence number. The target drops a datagram that is older than the last one that it used. The source sends a pointer datagram only if the position is different from the last datagram. It always sends one when forwarding starts and when the pointer enters the portal. A key event thus does not move the target pointer (milestone M10). The target injects only the newest position: if more than one pointer datagram waits to be handled, the target handles all of them in sequence but injects only the newest position of each session. Before the target handles the next message of the reliable channel, it handles the waiting pointer datagrams. Thus a button goes to the newest position (user, 2026-10-28; M14: a slow main loop delayed the pointer by up to 0.9 s). The source keeps at most 256 bytes of unsent pointer datagrams (about 8). If the path stops, it drops the oldest ones. No queue of pointer positions can grow without limit.
- **NET-3** Each peer has its own key pair. Both peers present a certificate, and each peer accepts the other only if its fingerprint is in the local trust list for that computer ID (ADR 0004). The user adds a fingerprint with `telescreen trust <computer-id> <fingerprint>`. No trust on first use.
- **NET-4** A peer must not accept an injection request only because the sender is on the LAN. A trusted fingerprint identifies a peer; it does not authorize it. The target refuses a session request with `NoRoute` if its config has no handoff route from that source to the device (user, 2026-10-27).
- **NET-5** The config does not hold keys, the trust list, or runtime state.
- **NET-6** Sessions have a generation number and a heartbeat. A lease is held by one session only. The local state of one peer alone must not decide the lease owner.
- **NET-7** On macOS, the network runs in the app process (ADR 0008). The app keeps `NSLocalNetworkUsageDescription` in `Info.plist`. Measured on 2026-11-04: an app that macOS starts (a launcher or `open`) gets "no answer from" on each dial until the user turns on Telescreen in the Local Network settings. The same binary started from a shell connects, because the shell is then the responsible process. A new install did not always show the request. The Local Network behavior of a new install on a different Mac is open (AC-25).
- **NET-8** Each pointer datagram has a send time: the microseconds on the monotonic clock of the source since the first pointer datagram of the session (M14). The peers do not share a clock. Thus a peer uses only the difference between two send times.
- **NET-9** The target records statistics of the pointer datagrams of each incoming session (M14). The statistics do not change the session.
  - Counts: received datagrams, used datagrams, late datagrams (dropped by NET-2), lost datagrams (sequence numbers between the first and the newest used one that did not arrive).
  - The send interval: the difference of the send times of two datagrams with consecutive sequence numbers.
  - The arrival interval: the time between the arrivals of two consecutive used datagrams.
  - A pause: an interval with a send-time difference of more than 100 ms (the pointer did not move). A pause is counted, and its interval is not in the interval statistics.
  - The arrival time is the time when the network thread receives the datagram, not the time when the main loop handles it.
  - The delay variation: for each received datagram, its arrival time minus its send time, minus the smallest such value of the session. It is not the one-way latency, because the peers do not share a clock.
  - The handling delay: for each received datagram, the time from its arrival to the time when the main loop handles it.
  - Injected datagrams: the used datagrams that the target injected (NET-2: the others had a newer datagram in the same batch).
  - Each interval, the delay variation, and the handling delay give p50, p95, p99, and the maximum (nearest rank).
  - Each series keeps its newest 65536 samples. The counts are for the whole session. The memory of the statistics does not grow with the session time.
  - The target writes a summary to the log when the session ends. The debug protocol returns the summary of the current session, else of the last session.
- **NET-10** A dial that fails does not retry automatically. The portal shows a candidate with no connection as "offline", and the candidate with a running dial as "connecting". While a candidate has no connection, the portal shows a "Refresh" button. The button is available unless all candidates with no connection have a running dial or search. It dials each offline candidate one time. It does not run `portal-match` again, and it does not start a handoff (user, 2026-11-05).
- **NET-11** One computer can have more than one address (user, 2026-11-05). `endpoint.host` is one host or a list of hosts, and each host is an IP address or a DNS name. A dial tries all known addresses of the peer. It starts the first address, and it starts each next address 250 ms later. It uses the first IPv4 address of each host. The first QUIC handshake that completes wins. Only the winner sends `Hello`, and the transport closes the other attempts. Thus a losing attempt never replaces the connection (the listener registers a connection only after `Hello`). A new address for a peer with a running dial joins that race. If all attempts fail, the failure gives the reason of each address.
- **NET-12** Discovery finds the address of a trusted peer with the mDNS responder of the system (ADR 0011, user, 2026-11-05). macOS uses Bonjour. Linux uses Avahi through D-Bus. Telescreen has no responder of its own. If the system has no responder, discovery is not available, a notice gives the cause, and dials to the configured hosts still work.
- **NET-13** Discovery is on for a computer only if its config section has `endpoint.discovery: mdns`. A computer with this key can omit `endpoint.host`. A discovery result is only a hint for an address. It does not identify a peer: the mutual TLS check of NET-3 identifies the peer.
- **NET-14** "Allow discovery" in the control window makes this computer discoverable for 10 minutes (user, 2026-11-05). The control window shows the button only if the section of this computer has `endpoint.discovery: mdns`, and it shows the remaining time. The user can stop it before the end. A new start of the app does not continue it. The computer registers one service of type `_telescreen._udp` with a random instance name, the port of its endpoint, and its fingerprint as the TXT key `fp`. The record has the host name of the computer, because the system responder adds it (measured on 2026-11-05).
- **NET-15** Each dial of the portal candidates starts a search if a candidate with no connection has `endpoint.discovery: mdns` (user, 2026-11-05): when the portal finds its device, and when the user selects "Refresh" (NET-10). The dial and the search run at the same time, and the first address that connects wins. A search runs for 30 seconds, and it ends when no such candidate remains. A candidate with a running search and no running dial shows "searching". The search sends no fingerprint. A candidate with no configured host and no known address has no dial.
- **NET-16** Telescreen uses a search result only if its `fp` is in the trust list, and only its private IPv4 addresses (RFC 1918 and link-local). Each computer that the trust list holds for this fingerprint gets the addresses. A new result replaces the previous result of that computer. The results are in memory only. A dial tries the found addresses first, then the configured hosts. A result for an offline candidate starts a dial. If a dial to this candidate runs, the found addresses join its race (NET-11).

## 8. Platforms

### 8.1 Backends for v0.1.0

| Capability | macOS | Linux (Wayland) |
| --- | --- | --- |
| Portal window | winit native fullscreen + AppKit window level | winit fullscreen window |
| Capture and suppress | CGEventTap at the HID location (Accessibility, Input Monitoring) | Window input of the portal + keyboard shortcuts inhibit |
| Pointer injection | `CGEventPost` | wlroots virtual pointer, absolute motion on the input-target output. Else a uinput absolute pointer (IN-15) |
| Key injection | `CGEventPost` | uinput keyboard (the compositor keymap applies) |
| Prompt window (VER-5) | AppKit window above the fullscreen level | layer-shell overlay |
| Focus holder (UI-7) | transparent key window | layer-shell surface, exclusive keyboard |
| Text clipboard (CLIP-8) | `NSPasteboard` | data-control protocol |
| Network | in process (ADR 0008) | in process |

### 8.2 Support matrix

Record each environment that passed the acceptance criteria. At this time only spikes ran (`docs/spikes.md`):

| Environment | Status |
| --- | --- |
| macOS 27.0.1, Apple silicon | Spikes 1, 2, 4 passed. Product not tested. |
| Linux, KDE Plasma with KWin 6.7.4 (Wayland), x86_64 | Mac to Linux: handoff, pointer, buttons, keys, scroll, and clipboard passed a manual test (user, 2026-11-04). One known problem: the top-left hot corner (issue 1). KWin 6.8.2: the button scroll of IN-15 passed a manual test with a trackpad (user, 2026-11-05). |
| Linux, niri 26.04 (Wayland), x86_64 | Mac to Linux and Linux to Mac: the clipboard hardware test passed with the build of IN-15, except session S3 (the Mac portal did not get focus in the test). The user did a short manual check of the other input (2026-11-04). |
| Linux X11, KDE, GNOME, sway, Hyprland, Windows | Not tested. Do not claim support. |

### 8.3 Distribution

- **DIST-1** v0.1.0 is built from source: a script makes an ad-hoc signed `Telescreen.app` on macOS, and `cargo install` on Linux.
- **DIST-3** v0.3.0 is the first binary release (user, 2026-11-04). A pull request from `main` to `release` runs the checks `linux` and `macos`, and they are required to merge it. `main` has no checks (user, 2026-11-05). The merge is a push to the `release` branch, and it runs `.github/workflows/release.yml`. It makes a GitHub release with `v<version>` from `Cargo.toml`: an archive for macOS arm64, an archive for Linux x86_64, `sha256sums.txt`, `install.sh`, and a `PKGBUILD`. The user installs with `install.sh`, with no sudo. The `PKGBUILD` is not published to the AUR. The macOS app has the self-signed "Telescreen Dev" signature if the secrets exist, else an ad-hoc signature. It is not notarized, so a download with a browser is blocked by Gatekeeper.
- **DIST-4** The Linux archive has a stripped binary and the launcher entry `dev.telescreen.Telescreen.desktop` (user, 2026-11-04). The file name is the Wayland app ID of the windows, so that a compositor can link a window to the entry. `install.sh` installs the entry in `~/.local/share/applications` with the absolute path of the binary, because the session of a launcher can have no `~/.local/bin` in its `PATH`. The `PKGBUILD` installs it in `/usr/share/applications`. The entry has no icon, because the project has no icon yet.
- **DIST-5** `scripts/make-macos-app.sh` does not put a local build in `target/` (user, 2026-11-04), because the launcher then shows two Telescreen apps. The default output is `~/Library/Caches/telescreen-dev.noindex`. Spotlight does not index a `.noindex` directory.
- **DIST-2** Before a binary release: test the real download path (dmg with quarantine, Gatekeeper, App Translocation) and each signing type that you ship (`docs/spikes.md`, spike 4).

## 9. Acceptance criteria

Each row must pass on each environment in the support matrix before you mark a version complete.

| ID | Scenario | Expected |
| --- | --- | --- |
| AC-1 | Desktop / Monitor1 / MyScreen use the same device `id` | Telescreen finds the same physical device. It does not compare the aliases |
| AC-2 | Two Linux computers share a second display. The Mac does not connect to it | The Mac is not a candidate for the second device |
| AC-3 | Select the Linux computer in the portal on the external display | The handoff runs on the Mac. The display shows the Linux computer. The prompt shows on the input-target display with the hint |
| AC-4 | Click the switch button | The click does not go to the target |
| AC-5 | Press Return in the focused portal while the prompt shows | Forwarding starts. The key does not go to the target. The device state is confirmed |
| AC-6 | Press Return with no prompt | It is a normal key. Forwarding does not start |
| AC-7 | No acknowledgement in 60 seconds | The wait stops. The prompt goes away. The device state is unknown. It does not switch back |
| AC-8 | The portal moves to a different display | `portal-match` runs again. The old binding and coordinates do not apply |
| AC-9 | `portal-match` false / more than one true / timeout | No match / ambiguous / error. No device is selected arbitrarily |
| AC-10 | A handoff step fails, or the verifier answers not ok | Forwarding does not start. No retry. No rollback claim |
| AC-11 | The pointer leaves the portal to the built-in display, then comes back, with no click | Pointer input is local while out; keys still go to the target; the session stays in forwarding. No key or button stays pressed on the target |
| AC-12 | Resolution, scale, rotation, negative coordinates, different aspect ratios | The target pointer position is correct. Out-of-bounds behavior is defined |
| AC-13 | Ordinary keys, modifier keys, and shortcuts | They have an effect on the target and not on the source |
| AC-14 | Space swipes, Mission Control, Command+Tab while forwarding on macOS | They do not happen on the source and do not go to the target |
| AC-15 | Hand back shortcut on the source | The session ends. Local input comes back. The target runs its route back, and the source shows the prompt (HB-3) |
| AC-16 | Escape 10 times in 5 seconds | Suppression and the session end, also when the main loop does not respond |
| AC-17 | System-reserved shortcuts, missing permission, Secure Event Input | Telescreen reports the limit. It does not claim capture |
| AC-18 | Disconnect, process crash, target hang, queue overflow | Both sides release all input state. Local control comes back |
| AC-19 | The portal display disappears from the source | Forwarding stops. The UI comes back. No input is suppressed in a moved window |
| AC-20 | Two sources try to control the same target device | The lease allows only one session |
| AC-21 | A peer with an unknown fingerprint dials, or is dialed | The connection is refused on both sides |
| AC-22 | Linux is the source and macOS is the target | AC-3 to AC-18 pass in this direction, with the Linux exits (shortcut inhibit) |
| AC-23 | The target selects "Disconnect" or "Control from here" | The session ends on both sides. With "Control from here", the session in the other direction starts |
| AC-24 | A generic USB device | The user can hand it off from the device list. It does not need to be a display |
| AC-25 | The installed macOS app connects on the LAN | The connection works (Local Network privacy) |
| AC-26 | While the source waits for the acknowledgement, try to switch the Space or the application, or click a different display | The portal stays fullscreen and keeps the focus (FLOW-21) |
| AC-27 | Type in a field on the target, click a field on the source, type, then click the field on the target again | After each click, only the clicked field shows the keyboard focus, and the keys go there (FLOW-22, FLOW-23). The click in the portal reaches the target |
| AC-28 | Hold Shift in the portal, then lose the keyboard focus without a key up (Linux: a compositor shortcut that cannot be inhibited) | The target releases Shift (IN-13). The session continues |
| AC-29 | A target with no focus holder backend | The session starts. One desktop notification and a warning in the control window (UI-7) |
| AC-30 | Switch on at both peers. Copy text on the source, then paste on the target; then copy on the target and paste on the source | The text arrives in both directions (CLIP-2) |
| AC-31 | Copy text before the session starts | The text is not sent (CLIP-3) |
| AC-32 | Switch off on one peer, or end the session, then copy | Nothing is sent (CLIP-2, CLIP-5) |
| AC-33 | Copy on one peer | The other peer does not send the same text back (CLIP-4) |
| AC-34 | Copy a password from a password manager, then wait until the password manager clears its clipboard | The password arrives with the concealed mark; then the other peer clears it (CLIP-7) |
| AC-35 | Copy text on one peer. End the session, or turn the switch off on the other peer | The other peer clears its clipboard. If the user copied something else there before, it stays (CLIP-11) |

| AC-36 | The target selects "Hand back" | The session ends. The target runs its route to the source. The source shows the prompt on the display; Return on the keyboard of the source acknowledges it. The device state is confirmed (HB-1, VER-4) |
| AC-37 | The target has no route back to the source | "Hand back" is disabled and shows the cause. The hand back shortcut ends the session, and the target shows the notice to switch by hand (HB-2, HB-4) |
| AC-38 | The portal leaves fullscreen while forwarding | The source requests a hand back (UI-3) |
| AC-39 | The device of the target has no `verify` | Forwarding starts after the steps. The device state is unverified (VER-2) |
| AC-40 | The verification fails or times out | The device state is unknown on the destination. "It is back" sets it to confirmed (VER-6) |
| AC-41 | Three peers: A has a portal on display D and a session to C. The user switches D to B by hand, then requests a session to A from B | A closes its portal on D. The session A to C ends. A shows "Session ended: <B> took this display" (FLOW-24) |
| AC-42 | A candidate is offline, and the user selects "Refresh" | One dial starts. The candidate shows "connecting". A second "Refresh" does not start a second dial, and the button is not available while the dial runs. A failed dial shows "offline" and the button again. A connected candidate becomes available (NET-10) |
| AC-43 | A peer has more than one address, and only a later address answers | The dial connects without a wait for the timeout of the earlier addresses. Each peer reports one connection (NET-11) |
| AC-44 | All addresses of a peer fail | One dial failure gives the reason of each address (NET-11) |
| AC-45 | A computer selects "Allow discovery", and the other computer opens a portal | The search finds the first computer in both directions, also when its configured host is wrong or absent, and the dial connects with no wait for the timeout of the wrong host (NET-15, NET-16) |
| AC-46 | 10 minutes pass after "Allow discovery", or the user stops it | The service is removed, and a new search does not find the computer (NET-14) |
| AC-47 | A search result has a fingerprint that is not in the trust list, or only public or IPv6 addresses | Telescreen does not dial it (NET-16) |
| AC-48 | Linux has no running Avahi | "Allow discovery" and the search give a notice with the cause. Dials to the configured hosts work (NET-12) |
| AC-49 | The control window is on one display, and the user selects "Open portal" on a different display | The portal opens on the selected display. The control window does not move. A portal on a third display closes (FLOW-2) |
| AC-50 | On KDE Plasma as the target, scroll in Chrome with the trackpad of the source, and stop | The page follows each scroll event, and it does not move after the scroll events stop (IN-15). A mouse wheel that is connected to the target keeps its usual behavior. Telescreen sends no middle click. The pointer shape does not change. With `disabled-quirks: [kwin-button-scroll]`, the scroll is a wheel scroll, and the device for button scroll does not exist |

Performance: measure p50, p95, and p99 of capture to injection, event loss, and idle CPU/GPU. A network latency number must state how it was measured (round trip on one clock, or the clock synchronization method). There is no hard threshold yet.

## 10. Open questions

- GNOME as the target (user, 2026-10-27: GNOME must be supported in the end). GNOME has no layer-shell, so the prompt (UI-5) and the focus holder (UI-7) need a different backend behind the same interfaces. A candidate is a GNOME Shell extension. X11 is not an option (user, 2026-10-27).

- The lease arbitration and the recovery protocol (NET-6): message design and timeouts. Tentative proposal for v0.1.0: `docs/adr/0007-session-protocol.md`. The user decided to keep it tentative until the hardware tests (AC-18) run.
- Which special shortcuts to support beyond IN-5, and the per-platform list of shortcuts that cannot be captured.
- The effect of Wi-Fi delay spikes (spike 4: p99 about 85 ms) on the pointer in real use. Measure it in the product.
- Minimum OS versions for each package. macOS: 27.0 for v0.1.0, the only tested version (user, 2026-10-27). Linux: open.
- Drivers for the output format of one tool (for example `betterDisplay` or `ddcutil`, with `expect`): a later version, after their output is measured on the hardware (user, 2026-10-27). The other driver decisions are in `docs/config.md`.
- A window that does not depend on the portal (a hub) is a different topic.

Decisions of the implementation (Claude, 2026-10-27). The user confirmed the items that name the user:

- The texts of the session end on the source (user, 2026-10-27), with the target label:
  - `TargetStopped` ("Disconnect" on the target): "<target> disconnected the session".
  - `TargetDisconnected` (the target app quit and told the source): "Telescreen quit on <target>".
  - `PeerLost` (heartbeat timeout or a closed connection): "Connection to <target> lost".
- "Control from here" (FLOW-17) needs a device match for the display (user, 2026-10-27). If no device matches, the target shows a notice, does not start a handoff, and the old session continues.
- CLIP-8: v0.2.0 uses only `zwlr-data-control-unstable-v1`. `ext-data-control-v1` needs a staging feature of `wayland-protocols` and can come later.
- The Linux clipboard backend reads an offer for at most 1 second. A text larger than 256 KiB is cut to its longest valid UTF-8 prefix, so that the core rejects it as too large (CLIP-6).
- The controller does not read the clipboard while the switch is off or while no session can sync (CLIP-2). On Linux, the data-control connection is open only while the switch is on and a session is in forwarding (user, 2026-10-27). At start, the app only checks that the compositor has data-control (CLIP-8). When the connection opens, the current selection is the old text of CLIP-3. When the connection closes, Wayland removes a text that Telescreen still provides (CLIP-11).
- Later goal: no stable identity on the network (user, 2026-11-05). See ADR 0011, "Later goal".
