# Technology Selection (2026-10-27)

## Recommendation: Rust core + winit windowing + egui UI + per-platform native backends

Rust is applicable to low-latency event handling, a background agent, and native API integration. You can make a separate Rust package for macOS, Linux, and Windows. Cross-platform means a shared core and a shared protocol. It does not mean that each OS can use the same input implementation. This is a recommendation for the selection. No on-hardware prototype in this project has validated it yet. No dependency versions are pinned.

winit manages OS windows and the event loop. egui supplies the target list and the settings UI. When necessary, integrate egui-winit/renderer directly to make sure that you have access to the native window. When the UI has no animation or interaction, it should use event-driven updates. This prevents constant GPU redraws. [egui official project](https://github.com/emilk/egui)

If a Web UI is preferred, Tauri + Rust can be an alternative. Its packaging tools are more complete. But a WebView keyboard handler or the global shortcut plugin cannot replace a complete input interception/injection backend. The UI of this project is simple. Thus, the preference is to not use a WebView layer. [Tauri Global Shortcut](https://tauri.app/plugin/global-shortcut/)

C++/Qt is also a possible option. But it adds C++ memory management and an evaluation of Qt distribution/licensing. Python is applicable to helpers or validation tools. But Python is not used for the primary resident input agent. These are engineering trade-offs for this project. They are not conclusions from performance benchmarks.

## Component Boundaries

- Core: config, shared ID resolution, state machine, leases, driver registry. The core does not identify specific displays.
- UI: dynamic portal windows, fullscreen, candidate computers, status, and the release-control entry point.
- Platform: input capture/suppress, inject, cursor, display topology, clipboard, permission/capability reporting.
- Transport: long-lived connections with pairing authentication and encryption. Session generation, sequence numbers, heartbeat, disconnect cleanup. An endpoint is not the same as an authenticated peer. v0.1.0 uses QUIC (ADR 0001). Pairing is manual fingerprint trust (ADR 0004).
- Driver: conditions, target area resolution, device handoff. Drivers must not be in the hot path of each input event.

OS event tap/hook callbacks do only the necessary capture and the decision whether to suppress the event. Then they send the event to a bounded queue. In the callback, do no network I/O, do not wait for helpers, and do not block the UI. You can coalesce mouse motion. You must not drop key/button down/up events arbitrarily. On queue overflow, you should stop the session and clean up the pressed state.

## Windows and Fullscreen

The goal is a portal that is borderless, has no title bar, and covers the target display. Decision (user, 2026-10-27, ADR 0003): on macOS the portal is a native fullscreen Space, so that the user can move to it with trackpad gestures. The portal hides the menu bar and the Dock completely with `hideMenuBar | hideDock` presentation options. While forwarding, the portal suppresses Space and application switching.

The winit macOS extension supplies `set_simple_fullscreen` and `set_borderless_game`. `set_borderless_game` can control the Menu Bar/Dock behavior in fullscreen. A window delegate can also return the options from `window:willUseFullScreenPresentationOptions:`. It is not guaranteed that the system UI stays hidden on all macOS versions and multi-display configurations. Spike 1 verifies it. [winit macOS extension](https://docs.rs/winit/latest/winit/platform/macos/trait.WindowExtMacOS.html)

Use AppKit presentation options when necessary. `hideMenuBar`/`hideDock` are different from auto-hide. With auto-hide, the Menu Bar/Dock can still show when the cursor comes near the edge. AppKit sets constraints on the combinations. You must save/restore the app presentation state on entry, on exit, on loss of focus, and on abnormal conditions. This prevents an effect on the work of the user on another display. [Apple PresentationOptions](https://developer.apple.com/documentation/appkit/nsapplication/presentationoptions-swift.struct)

Acceptance testing must actually test these conditions: the mouse at the top edge, the Dock edge, Hot Corners, movement back and forth between the built-in display and the external display, and Spaces switching. You must not treat decorations=false as a verification that "no system control bar will ever appear". Keep a release-control path that is available locally.

## Input and Shortcuts

In the requirements, "Capture" has four parts: listening, prevention of local handling, forwarding, and remote injection. Probe the four parts separately. A global hotkey registration is not the same as suppression of arbitrary shortcuts.

| Platform | Candidate implementation | Boundaries |
| --- | --- | --- |
| macOS | CoreGraphics event taps / event posting, AppKit windows, native cursor APIs when necessary | Needs the corresponding Accessibility/Input Monitoring authorization. Secure Input can make interception unavailable. Thus, it is not possible to promise all key combinations |
| Windows | Low-level keyboard/mouse hooks, SendInput | Hooks must return quickly. The integrity level/UIPI restricts injection. The commitment does not include the secure desktop and the secure attention sequence |
| Linux X11 | X11/XInput, grabs, and XTest as candidates | Must handle existing grabs and focus conflicts. If you can read raw events, this alone does not mean that you can suppress events |
| Linux Wayland | InputCapture portal; RemoteDesktop portal/libei for injection | Depends on the actual capabilities and authorization of the compositor/portal backend. Availability on all desktops cannot be guaranteed. You also cannot assume that arbitrary global coordinates are available |

Apple documents event taps and Secure Event Input explicitly. But you must still test permissions and OS special cases on the target versions. [CGEvent tap](https://developer.apple.com/documentation/coregraphics/cgevent/tapcreate(tap:place:options:eventsofinterest:callback:userinfo:)), [Secure Event Input](https://developer.apple.com/library/archive/technotes/tn2150/_index.html)

Windows low-level hooks have a time limit. SendInput allows injection only into programs at the same or a lower integrity level. [LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc), [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)

Wayland requires capability negotiation. The activation of InputCapture uses barriers that the compositor manages. This is not the same as hover events on an arbitrary window. The full portal hover mode needs a separate verification. When capabilities are missing, you must show the reason. [InputCapture protocol](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.InputCapture.xml), [RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html), [libei](https://libinput.pages.freedesktop.org/libei/)

Input events are mainly physical key identity + down/up. Text input/IME is a separate topic. At first, use the input method of the remote computer. By default, do not translate macOS Command into Ctrl. Keep at least one local release shortcut that is never forwarded. Suppression is permitted only while a session is active and has input focus. At all other times, send input to the OS normally.

## Cursor, Focus, Clipboard

Both computers keep a shared virtual pointer state. input-target selects a display on the receiving computer. The source portal position is normalized and then mapped to this display. The backend handles scaling, rotation, negative origins, and pixel coordinates. You must implement relative motion and edge-exit together. The trigger of an ordinary mouse-leave must not rely on the locked OS cursor. The remote target is limited to the selected display. It must not unexpectedly cross the boundary and control other displays.

The hidden cursor/virtual position on the source computer and the actual cursor on the target must keep a predictable boundary relationship. An exit from the area restores the source cursor and input. A focus change, a disconnect, a permission revocation, and a display unplug each release all held keys/buttons. The receiving computer must clean up by itself through a heartbeat timeout. It must not rely only on a key-up from the source computer.

The clipboard uses platform backends. Start with text only. Use it only for paired and active sessions. It is enabled explicitly from the UI. Stop it on disconnect. Deduplicate by source ID/version. This prevents the two computers from writing back to each other. Size limits, the format list, and error handling must be explicit. Adapt macOS NSPasteboard, the Windows Clipboard API, and Linux clipboard/portal separately. You must validate Wayland background clipboard permissions together with session capabilities. Images and files are future scope.

## Packaging and Validation Priorities

macOS: native .app with a signing/notarization release pipeline. Windows: .exe and installer. Linux: an evaluation of native packages or AppImage. Linux system libraries, Wayland portals, and permissions do not disappear automatically through single-file packaging. Each OS/CPU gets its own CI build/test. It is not possible to promise a single executable for all three platforms.

First, make a prototype of macOS borderless + input interception and of Linux injection on the receiving computer. Then verify that the connection stays after a physical display switch. If the OS of the source computer removes the external display, it is possible that the window moves to the built-in display. The initial version should stop forwarding and supply a recovery UI. It must not assume by default that the hidden portal stays at its original coordinates. Virtual displays/EDID emulators can be evaluated separately. They are not part of the current commitment.
