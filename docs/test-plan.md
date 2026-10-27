# Hardware test plan for v0.1.0

This plan runs the acceptance criteria of `docs/spec.md` section 9 on the hardware. The automated tests do not replace it. Record each result in `docs/spikes.md` style: the date, the environment, pass or fail, and what the user saw. Update the support matrix (spec section 8.2) only for criteria that passed.

Rules:

- The user does each step that injects keys, clicks, opens a window, or switches the display. Before such a step on the Linux computer, the user moves to a workspace that the step does not affect.
- Each step names what it does. If a step does something that the plan does not say, stop and record it.
- Keep a way back: the release shortcut, the safety exit (Escape 10 times in 5 seconds), and on niri `Mod+Shift+Escape` (toggles the shortcut inhibit; it always goes to niri).

## 0. Build and install

On the Mac:

```sh
scripts/make-macos-app.sh
```

It builds `target/release/Telescreen.app`, ad-hoc signed. It does not start it.

On the Linux computer (in a checkout of the same commit):

```sh
nice -n 19 cargo build --release -j2 -p telescreen-app
```

Install `contrib/display-helper` on both computers at the path that the config uses, and make it executable. Copy `contrib/example-lg.yaml` to `config.yaml` and change the host names. On the current hardware, neither computer resolves the `.local` name of the other (measured 2026-10-27), so use IP addresses. `telescreen displays` shows the display metadata for the helper arguments. Check it on both computers:

```sh
telescreen check-config config.yaml
```

## 1. Pairing (AC-21)

On each computer: `telescreen fingerprint`. Then on the Mac `telescreen trust linux <fingerprint of Linux>`, and on Linux `telescreen trust mac <fingerprint of the Mac>`. These commands only write the local trust file.

Negative test (AC-21): before the trust step, start both apps (step 2). Each control window must show a refused or failed connection, and no session can start.

## 2. Start

- Mac: `open -a target/release/Telescreen.app --args run --config "$PWD/config.yaml" --computer mac`. macOS asks for Accessibility and Input Monitoring for Telescreen. Grant them, quit, and start again. macOS can also ask if `telescreen` can accept incoming network connections (the application firewall). Allow it, else Linux cannot dial the Mac (part 4). Expected: the control window shows the displays, and no "not available" notice for capture or injection.
  - Sign the app with a stable identity, else each build needs new grants: make a self-signed code signing certificate with the name "Telescreen Dev" in the login keychain (`openssl req -x509` with `extendedKeyUsage = critical, codeSigning`, then `security import ... -T /usr/bin/codesign`). `scripts/make-macos-app.sh` uses it if it is present. Use `/usr/bin/openssl` (LibreSSL): macOS cannot import a `.p12` file from OpenSSL 3 or later with the default options.
  - An old grant for a different signature blocks the new grant. The TCC log then says "Failed to match existing code requirement". Run `tccutil reset Accessibility dev.telescreen.Telescreen` (and `ListenEvent`), then grant again.
  - On macOS 27 in zh-TW, the Accessibility list is "隱私權與安全性 › 裝置控制和資料取用" (the English name is not checked).
  - `open` on a running app does not start it again. Quit the app first, then check that the process stopped.
- Linux (opens a window; move to a free workspace first): `target/release/telescreen run --config config.yaml --computer linux`. Expected: the control window, no notice about injection. A notice that `/dev/uinput` cannot be opened means a permission problem.

## 3. Mac to Linux (M4, M5, M6)

| Step | Action | Expected | AC |
| --- | --- | --- | --- |
| 3.1 | Mac control window on the built-in display: "Open portal on this display" | Matching says no device matches. No candidate | AC-9 |
| 3.2 | Move the control window to the LG: "Open portal on this display" | A fullscreen Space on the LG. No menu bar or title bar at the top edge. Input source is ABC while it has the focus. "LG" matched. Candidate "Linux" available after the connection | UI-1, UI-2, FLOW-2, FLOW-3 |
| 3.3 | Click "Linux" (switches the display; Linux gets the prompt) | The LG shows Linux. The prompt with the hint shows on the LG. The Mac portal says "Press Return or click". The click did not go to Linux | AC-3, AC-4 |
| 3.4 | Before Return: three-finger swipe, Command+Tab, and a click on the built-in display | The portal stays fullscreen and keeps the focus | AC-26 |
| 3.5 | Focus the portal, press Return | The prompt goes away. The Mac cursor stays free. Pointer and keys control Linux. Video input "confirmed". Return did not go to Linux | AC-5 |
| 3.6 | Type text, use Shift, Control, Super (Command), arrows | Keys work on Linux, not on the Mac | AC-13 |
| 3.7 | Move the pointer to all four corners of the LG | The Linux pointer reaches each corner and stays on the LG output. The top-left corner opens the niri overview: test it last, or skip it | AC-12 |
| 3.8 | Three-finger swipe, Mission Control, Command+Tab | Nothing happens on the Mac. Nothing goes to Linux | AC-14 |
| 3.9 | Hold Shift, move the pointer out of the portal edge to the built-in display, release Shift | Pointer input goes to the Mac. The keys still go to Linux (no click yet). Shift is not held on Linux or on the Mac. The LG still shows Linux | AC-11, IN-3, FLOW-22 |
| 3.9a | Hold Shift on the built-in display, move the pointer into the portal, release Shift | Shift is not held on the Mac or on Linux | IN-3 |
| 3.10 | Move the pointer back into the portal | The pointer controls Linux again, with no prompt | AC-11 |
| 3.10a | Type in a Linux text field. Click a text field on the built-in display, type, then click the Linux text field again and type | After each click, only the clicked field shows a text cursor, and the keys go there. The click in the portal reached Linux | AC-27, FLOW-23 |
| 3.10b | Copy text on the Mac, paste on Linux; copy on Linux, paste on the Mac | The text arrives in both directions. It does not come back | AC-30, AC-33 |
| 3.10c | Turn off "Sync text clipboard" on one side, then copy | Nothing arrives | AC-32 |
| 3.10d | Copy a password from a password manager that marks it as concealed | Nothing arrives | AC-34 |
| 3.11 | Control+Option+Command+Escape | The session ends. Local keys come back | AC-15 |
| 3.12 | New session, then Escape 10 times in 5 s | The session ends; Escape went to Linux | AC-16 |
| 3.13 | New session, then on Linux "Stop this session" | The session ends on both sides | AC-23 |
| 3.13a | New session, then on Linux "Control from here" | The session ends. The LG switches to the Mac, and Linux is the source of a new session | AC-23, FLOW-17 |
| 3.14 | New session, then stop the Linux app (or unplug the network) | In 5 s the Mac gets control back. Linux has no stuck key | AC-18 |
| 3.15 | Start a session and do not confirm for 60 s | The prompt goes away. Video input "unknown". The display does not switch back | AC-7 |
| 3.16 | Switch the display back to the Mac by hand (BetterDisplay or the monitor buttons) | (Telescreen does not switch back: FLOW-18) | — |

## 4. Linux to Mac (M7, AC-22)

Before this part: the Linux handoff command (`ddcutil` with `--i2c-source-addr=x50`) is not tested. Test it alone first, with the user watching the display: run the `sh -c` command of the config by hand on Linux. It must switch the LG to the Mac and exit with 0.

Repeat the table of part 3 with the roles reversed: the portal on Linux (a fullscreen window on HDMI-A-2), the prompt on the Mac (an overlay window). Differences:

- The release shortcut is Control+Alt+Shift+Escape.
- While forwarding, niri shortcuts (Mod+…) go to the Mac, because the portal inhibits them. `Mod+Shift+Escape` always goes to niri.
- Linux has only one output, the LG. The pointer cannot leave the portal, so the only way to give the input back to Linux is the release shortcut.
- Not tested yet: the shortcut inhibit on niri.

## 5. Other criteria

- AC-17: on the Mac source, focus a password field (Secure Event Input) and start forwarding. Expected: a notice, no capture claim.
- AC-28 (Linux source): hold Shift in the portal, press `Mod+Shift+Escape` (niri does not inhibit it), then change the focus with a niri shortcut. Expected: Shift is released on the Mac.
- AC-29: start a target on a compositor with no layer-shell (for example GNOME). Expected: a notice, and a refusal with "no keyboard focus holder".
- AC-31: copy text before the session starts. Expected: it does not arrive.
- Unattended checks with the debug protocol (no clicks, no keys): `hold_keyboard` on Linux makes the `focused` fact of the control window false; `release_keyboard` makes it true again.
- AC-19: unplug the LG (or turn it off) while forwarding. Expected: forwarding stops and the portal closes.
- AC-20: needs a third computer. Not possible with this hardware.
- AC-24: needs a USB switch. Not possible with this hardware.
- AC-25: install the app from a downloaded dmg (with `com.apple.quarantine`), drag it to Applications, and start it. Expected: macOS shows the Local Network prompt, and after the user allows it, the connection works. A local build does not test this path (ADR 0008).
- Performance (spec section 9): not measured in v0.1.0 yet.

## 6. Results

### 2026-10-27, Mac to Linux, niri

- Not tested: the release shortcut (3.11). The user skipped it.
- Fail (open bug): Alt+Tab on niri while forwarding. The niri window switcher selects the window under the pointer, although the user did not move the mouse.
  - Cause: the macOS capture reports the cursor position on each event, also on key events (`telescreen-platform-macos/src/capture.rs`, `handle`). While forwarding, each key thus sends a pointer datagram with the same position.
  - The Linux injector sends each datagram as `motion_absolute` of the virtual pointer. Nothing removes a duplicate position, so niri gets a pointer motion for each key.
  - Proposed fix: the source controller sends a pointer datagram only if the position changed. It always sends one when forwarding starts and when the cursor enters the portal. The pointer channel is not reliable: if the last datagram is lost, the target pointer stays at the old position until the next motion.
- Not reproduced: once, the prompt went away before the confirmation. The user thinks that a click out of the portal, or display sleep, can be the cause. In the Mac log, one Linux to Mac session ended with "heartbeat timeout" after the prompt showed. A disconnect ends the session, and then the prompt goes away (FLOW-16). The cause is not known. See milestone M11 in `docs/plan.md`.
