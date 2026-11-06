# 0012: Native UI on each platform, with the Rust main loop

Status: accepted (user, 2026-11-06). Spikes 6 and 7 confirmed the open points, except niri (see "Open points").

## Context

egui is too limited for the new UI (user, 2026-11-06): a menu panel, a main window, settings, and later a setup wizard and pairing. The menu must look like a system menu. On macOS 27 this means the system glass material and the system transparency settings.

Options that we examined:

- GPUI: it has only a blur of the whole window, and its single-pass renderer has no backdrop blur of elements. The user does not want it (2026-11-06).
- Slint: a stable 1.x API, but from 1.16 the default style is Fluent on all platforms, and the native-looking styles will be deprecated. Thus it does not look native.
- Freya: version 0.4 (2026-07) is a rewrite, and 0.5 is a release candidate. The API is not stable.
- Iced: 0.x with breaking changes in each release, and a slow release cadence. The default look is plain.
- Web-based toolkits: rejected by the user (2026-11-06).

The macOS menu needs Swift in any case: the Control Center button is a WidgetKit extension. The KDE menu needs QML in any case: a Plasma applet is QML.

## Decision

- Each platform has a native UI. macOS uses SwiftUI. Linux uses Qt/QML through cxx-qt. GTK is not part of this decision (user, 2026-11-06).
- Rust owns the main loop (user, 2026-11-06). The Rust `main` starts the run loop of the platform: `NSApplication` on macOS (through objc2), `QGuiApplication` on Linux (through cxx-qt). SwiftUI views live in `NSHostingView` in windows that Rust makes. Rust calls Swift through a C ABI (for example UniFFI).
- All logic stays in Rust: the core, the controller, and the view models in `telescreen-view`. A UI only draws a view model and sends intents back. A UI makes no decision.
- A view model has a stable id for each row (for example `ComputerId`), so that a retained UI can find the changes. A view model can be serialized, because the KDE applet reads it over D-Bus.
- The portal becomes a plain native window with no drawn content, or one status line.
- winit and egui stay until the native UI replaces them. Then they are removed.

## Consequences

- Each screen is drawn two times: one time in SwiftUI and one time in QML. The KDE applet and the Linux windows can share QML components.
- The macOS build needs a Swift toolchain. The Linux build needs Qt 6. The macOS app does not include Qt.
- The renderer contract changes from "draw each frame and return intents" to "push a view model when it changes, and receive intents through a callback".
- Features of a SwiftUI `App` that need a Swift entry point are harder. Example: App Intents need metadata that Xcode makes. The Shortcuts action is deferred for this reason (user, 2026-11-06).

## Open points

- Linux: can a cxx-qt window give its `wl_surface` to the keyboard shortcuts inhibit protocol (UI-6), on niri and on KWin? KWin: yes (spike 6, 2026-11-07), through a private Qt API. niri: not tested.
- macOS: does the data flow with `NSHostingView` and the C ABI work well with the Rust main loop? Yes (spike 7, 2026-11-07): a plain C ABI with JSON snapshots. The Control Center extension in a bundle with a Rust main is not tested.
