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

Known from a different project of the user (not-enough-tokens, an ad-hoc signed Electron app installed from a dmg): the installed app never got the Local Network prompt, it was not listed in System Settings, and LAN connections failed with "No route to host". This is different from the local build above, which got the prompt. The fix in that project: start the network process through a small helper that calls the private `responsibility_spawnattrs_setdisclaim` (weak-linked) with `POSIX_SPAWN_SETEXEC`. Then the network process is its own responsible process, and macOS does not apply Local Network privacy to command-line tools. That project also installs with `curl | sh`, because files from `curl` do not get `com.apple.quarantine`.
