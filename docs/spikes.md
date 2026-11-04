# Spike results

Each spike is throwaway code in `spikes/<name>/`. This file records what each spike showed. Do not add results that were not measured.

## Spike 1: macOS portal window (2026-10-27, macOS 27.0.1, winit 0.30.13)

Code: `spikes/portal-window/`.

| Question | Result |
| --- | --- |
| Native fullscreen Space on the external display | Works. winit `Fullscreen::Borderless(Some(monitor))` with `with_borderless_game(true)` makes a fullscreen Space. Presentation options are `0x40a` (`FullScreen \| HideDock \| HideMenuBar`). |
| Menu bar and title bar at the top edge, presentation options only | Fail. When the pointer touches the top edge, the menu bar and the title bar (with the window buttons) show, also with `HideMenuBar`. |
| Menu bar and title bar at the top edge, window level 26 | Pass (user observation). After the window enters fullscreen and has the focus, `setLevel(26)` (above `NSMainMenuWindowLevel` = 24) puts the portal above the menu bar and the title bar. They do not show. |
| Space change detection | Works. A swipe to a different Space gives `Occluded(true)` and `Focused(false)`. A swipe back gives `Occluded(false)` and `Focused(true)`. |
| Confirmation key in the window | Works. Return arrives as `KeyboardInput` with `PhysicalKey::Code(Enter)`. |
| Input method | With Zhuyin active, `q` arrives as logical `ㄆ` (physical `KeyQ`). Use physical keys. With `NSTextInputContext.allowedInputSourceLocales = [NSAllRomanInputSourcesLocaleIdentifier]` on the focused view, the input source is `com.apple.keylayout.ABC`, and letters arrive as Latin characters. After the spike quit, the input source was Zhuyin again (user observation). Thus macOS restores the previous input source. |
| Mission Control drag | In Mission Control, the user can drag the portal out of fullscreen. The window becomes a normal window (`Resized` to a smaller size). The product must detect it. |
| Display names | winit names the monitors `Monitor #<model>` (for example `Monitor #30457`), not the BetterDisplay name. Do not find the portal display by the BetterDisplay name. |

## Spike 2: macOS input capture and suppress (2026-10-27, macOS 27.0.1)

Code: `spikes/input-tap/`. An ad-hoc signed `.app` bundle, so that Accessibility and Input Monitoring go to this bundle only. A rebuild can change the code signature, and then macOS can ask for the permissions again.

| Question | Result |
| --- | --- |
| Event tap location | `CGEventTapCreate` at the HID location (`kCGHIDEventTap`) works with Accessibility and Input Monitoring. No root. |
| Keyboard | Key down/up (types 10/11) and modifier changes (type 12) arrive in pairs, with the keycode and the flags. Command+Tab, Control/Option/Command+arrows, and Command+Space arrive. |
| Suppress | The callback returns NULL. The user confirmed: Space swipes, Mission Control, and Command+Tab did not happen. Typed text did not arrive in a focused text field. |
| Trackpad | Gesture events (type 29) and type 30 arrive and are suppressed. Type 14 (system-defined) arrives with each click. |
| Cursor | `CGAssociateMouseAndMouseCursorPosition(false)` plus `CGDisplayHideCursor` freeze and hide the cursor. Mouse deltas come from fields 4 and 5. |
| Safety exit | Escape 10 times in 5 s, counted in the callback, works. |
| Release shortcut | Control+Option+Command+Escape works. |
| Crash | `abort()` while suppressing: input and the cursor came back at once. No crash dialog showed (user observation). |
| Deadline | Bug in the spike: the callback checks the deadline only when an event arrives. The product must check the deadline on an independent timer. |
| Secure Event Input | Not tested. With Secure Event Input on, an event tap does not see keys. The product must detect it (`kCGSSessionSecureInputPID` in the IORegistry) and show it. |

## Spike 3: Linux pointer injection on niri (2026-10-27, niri 26.04)

Code: `spikes/linux-inject/`. Built on the receiving computer.

| Question | Result |
| --- | --- |
| Protocols | niri has `zwlr_virtual_pointer_manager_v1` (version 2), `zwp_virtual_keyboard_manager_v1`, and `zwlr_layer_shell_v1`. Portal injection is not available (see `docs/hardware.md`). |
| Absolute pointer | `create_virtual_pointer_with_output` plus `motion_absolute` with an extent of 10000x10000 puts the pointer at the given fraction of that output. The user confirmed the center, the 98% edges, the bottom-right corner, and the bottom-left corner. Thus the receiving computer does not need to read the pointer position. |
| Click | `button(BTN_LEFT, pressed/released)` plus `frame` clicks. The user confirmed a click on a button at the bottom-left corner. |
| uinput pointer | A uinput device with `REL_X`/`REL_Y` and `BTN_LEFT` moves the pointer (user confirmed). |
| Not tested | Keyboard injection (uinput or virtual keyboard), scroll, and the layer-shell prompt. |

Decision for v1: the pointer uses the virtual pointer with absolute motion on the input-target output. Keyboard injection is not decided (uinput uses the niri keymap; the virtual keyboard needs its own keymap).

## Spike 4: QUIC transport Mac to Linux (2026-10-27, quinn 0.11, rustls 0.23 ring)

Code: `spikes/transport/`. The Mac is on Wi-Fi (`en0`). The Linux computer is on the same LAN. Round trips use the clock of the client only, so no clock synchronization is necessary.

| Question | Result |
| --- | --- |
| Pinning | A custom `ServerCertVerifier` that compares the SHA-256 of the server certificate with a pinned value works. A wrong pin fails the handshake with "fingerprint mismatch". |
| Connect time | 6.4 ms from a shell. |
| Reliable stream, 16 bytes, 100 Hz, 1000 round trips | p50 3.0 ms, p95 4.7 ms, p99 85 ms, max 94 ms, 0 lost. |
| Datagram, 16 bytes, 125 Hz, 1000 round trips | p50 2.9 ms, p95 4.0 ms, p99 89 ms, max 93 ms, 0 lost. Maximum datagram size 1162 bytes. |
| ICMP ping (20 packets) | min 2.7 ms, avg 11.8 ms, max 71.7 ms. Thus the p99 delay comes from the Wi-Fi link, not from QUIC. |
| Local Network privacy, app bundle without `NSLocalNetworkUsageDescription` | macOS showed the Local Network prompt. The connection completed after the user allowed it (7.0 s, the wait for the user). From a shell, the tool uses the permission of the parent app, so a shell test does not show this prompt. |

Release checks (not done; do them before a release):

- Test the real distribution path: a `.dmg`, downloaded (with `com.apple.quarantine`), dragged to Applications, then started. Ad-hoc signed apps from a download meet Gatekeeper and App Translocation. A local build does not show these effects.
- Test each signing type that you ship (ad-hoc, Developer ID). macOS keys the permissions (Accessibility, Input Monitoring, Local Network) to the code signature. An ad-hoc signature can change at each build, so an update can lose the permissions.
- Always put `NSLocalNetworkUsageDescription` in the app `Info.plist`.

Known from a different project of the user (not-enough-tokens, an ad-hoc signed Electron app installed from a dmg): the installed app never got the Local Network prompt, it was not listed in System Settings, and LAN connections failed with "No route to host". This is different from the local build above, which got the prompt. The fix in that project: start the network process through a small helper that calls the private `responsibility_spawnattrs_setdisclaim` (weak-linked) with `POSIX_SPAWN_SETEXEC`. Then the network process is its own responsible process, and macOS does not apply Local Network privacy to command-line tools. That project also installs with `curl | sh`, because files from `curl` do not get `com.apple.quarantine`. Note: the disclaimed process there is `/usr/bin/ssh`, a system command-line tool. This result does not apply to an executable in the Telescreen app bundle (ADR 0008).

## Spike 5: wheel scroll and button scroll in Chrome on KWin (2026-11-05, KWin 6.8.2, libinput 1.32.0, Chrome 156 on Wayland)

Code: `spikes/scroll-source/probe.py`. A script on the target makes the uinput devices and sends one fixed pattern. A page in Chrome records the wheel events and the scroll position of each frame. The values show the time from a wheel event in the page to the scroll position in the page. They do not show the time on the network or on the display.

| Question | Result |
| --- | --- |
| Scroll source on KWin with `org_kde_kwin_fake_input` or libei | From the source code, not measured: KWin sends no `axis_source`. Chromium then uses `WHEEL`. These paths do not remove the animation. |
| Default scroll method of a uinput mouse with `BTN_MIDDLE` and no wheel | KWin reports `scrollOnButtonDown=true` and `scrollButton=274` (`BTN_MIDDLE`). |
| Wheel device, 6 high-resolution units each 13 ms | Lag at half of the distance: 100.1 ms. The page continues to move for 189.5 ms after the last wheel event. |
| Button scroll device, 6 counts each 13 ms | Lag at half of the distance: 0.1 ms. The page continues to move for 0.1 ms after the last wheel event. |
| Linearity of button scroll at 1, 6, and 30 counts each 13 ms | 10.909 CSS pixels for each count at all three speeds. The wheel device gives 0.909 CSS pixels for each unit. One count is thus a large step for a slow scroll. |
| Slow horizontal scroll after a vertical scroll | Works, 1 count each 13 ms. The script first moves 5 counts and -5 counts on both axes while the button is down. |
| Middle button events in the page | None during the tests, and none after the button up. |
| Pointer shape while the button is down | User observation with the product build: KWin shows a scroll cursor, and it stays until the button goes up. From the source code: KWin shows it only if its `scrollOnButtonDown` is on. libinput applies a method change only when no button is down. |
| Real trackpad on the product build, with the KWin setting turned off while the button is down | User observation: the pointer shape is normal, and the scroll feels normal. |
| Other browsers and Qt applications, each one on its own | Not reported. |
| Button state after a compositor or session restart, a VT change, or a screen lock | Not tested. |
