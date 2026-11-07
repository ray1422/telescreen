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

## Spike 6: Wayland shortcut inhibit from a cxx-qt window (2026-11-07, KWin 6.8.2 and nested niri 26.04, Qt 6.12.0, cxx-qt 0.10.0)

Code: `spikes/qt-inhibit/`. Built and run on the Linux computer over SSH, with `QT_QPA_PLATFORM=wayland`. No key and no click.

| Question | Result |
| --- | --- |
| Wayland platform | `QGuiApplication::platformName()` is `wayland`. |
| `wl_display` | Public API: `nativeInterface<QNativeInterface::QWaylandApplication>()->display()`. |
| `wl_surface` of a QML `Window` | Private API: `QWindow::nativeInterface<QNativeInterface::Private::QWaylandWindow>()->surface()`, from `QtGui/qpa/qplatformwindow_p.h`. The build adds the versioned private include directories of QtCore and QtGui (`qmake6 -query`). A Qt update can change this API. |
| Second binding of the protocol | QtWaylandClient 6.12 does not bind `zwp_keyboard_shortcuts_inhibit_manager_v1` (no such string in the library). Thus the inhibitor of Telescreen is the only one for the surface. |
| Inhibitor on KWin | Works. The connection of Qt is a foreign display for `wayland-client` (the same as `inhibit.rs`). `inhibit_shortcuts` gets the `active` event about 100 ms later. The window was active (`QWindow::isActive`) with no click, also when SSH started it. |
| Events on the foreign display | A private event queue gets its events only when Telescreen dispatches it. The spike calls `dispatch_pending` from a Qt timer every 100 ms. |
| Hide, show, inhibit again | Works: `active` again. The surface pointer was the same after the show. Get the surface again before each request in any case. |
| Release and request again (UI-6) | Works: `active` again. |
| Exit | Fail with the first order: the process dumped core at exit when the Wayland proxies dropped after `QGuiApplication`. Pass when Telescreen drops all its Wayland objects before Qt closes the display. |
| Shortcut actually blocked | Not tested. It needs a key press with the user. |
| niri | Works in a nested niri 26.04 (an empty config, in a window of the KWin session): `active` after each request, also after hide and show, and a clean exit. The window had the focus in niri with no click. A niri session on the hardware is not tested. |
| cxx-qt bridge | A `#[cxx_qt::bridge]` does not accept a function pointer argument. The spike uses an `extern "C"` Rust function for the Qt timer. |

## Spike 7: SwiftUI in a window of the Rust main loop (2026-11-07, macOS 27, Swift 6.4, objc2 0.6)

Code: `spikes/swiftui-host/`. `build.rs` compiles one Swift file into a static library with `swiftc -emit-library -static`. The Swift code exports C functions with `@_cdecl`. Plain C ABI with JSON, no UniFFI.

| Question | Result |
| --- | --- |
| Link | Works with link search paths to the Swift libraries of the toolchain and the SDK, and an rpath to `/usr/lib/swift`. The binary uses the Swift runtime and SwiftUI of the system. |
| Run loop | The Rust `main` makes `NSApplication` (accessory policy), an `NSPanel`, and an `NSStatusItem`, and calls `run`. Swift makes the `NSHostingView` and returns it retained. |
| Push from a background thread | Works. Swift decodes the JSON on the caller thread and changes an `@Observable` model on the main thread (`DispatchQueue.main.async`). |
| Push to SwiftUI update | 60 pushes per second: 100 of 100 snapshots reached `onChange`. Push to `onChange` p50 3.5 to 4.0 ms, p95 4.5 to 5.5 ms, max 11 ms (4 runs). |
| Burst | 20 pushes with no wait: `onChange` saw 2 of them, the last one included. SwiftUI shows the newest snapshot and skips old ones. A view model must be a full state, not a change. |
| Intent to Rust | Works. The C callback runs on the main thread. Tested with intents that the view sends from `onChange` and `onGeometryChange`. A click on a button was not tested. |
| Window size | `fittingSize` before the first layout is too small (44 pt). The view reports its size with `onGeometryChange`, and Rust sets the panel frame. The first report can arrive before Rust keeps the panel, so Rust keeps the last size and applies it. |
| Active controls | A borderless `NSPanel` cannot become key by default, and then SwiftUI draws the buttons as inactive. A subclass with `canBecomeKeyWindow = true` (as `focus.rs`) fixes it. |
| Glass | `glassEffect` in the panel shows the desktop behind it (screenshot of the screen region). A screenshot of the window only (`screencapture -l`) shows no backdrop. |
| Not tested | A click on the status item, the Control Center extension (WidgetKit) in a bundle with a Rust main, and an app bundle build. |

## Spike 8: a Control Center button with no Xcode (2026-11-07, macOS 27, Command Line Tools only)

Code: `spikes/control-center/`. `build.sh` compiles one Swift file with `swiftc` into a WidgetKit extension, puts it into `Contents/PlugIns/` of the dev `Telescreen.app`, and signs it with the sandbox entitlement.

| Question | Result |
| --- | --- |
| Build with no Xcode | Works. `swiftc -parse-as-library -application-extension` with `-Xlinker -e -Xlinker _NSExtensionMain`. Without the entry point, the extension stopped at once in `_EXRunningExtension._shared` (ExtensionFoundation): `ExtensionMain.shared` had no extension type. Xcode adds this entry point. An installed widget (Tailscale) has it too. |
| Registration | `pluginkit` registers the extension after `lsregister -f -R` of the app. chronod reads the control descriptor (`kind`, `OpenURLIntent`, `isLauncher: YES`). |
| Signing | The self-signed "Telescreen Dev" identity is sufficient. The extension has no team identifier and no provisioning profile. An ad-hoc signature also stopped in the same place before the entry point fix, so the signature was not the cause. |
| Control Center gallery | The control shows in "Add a Control" (user, 2026-11-07). |
| Button action, system intent | Fail. With the system `OpenURLIntent`, a press fails: `Failed to create LinkAction`, then `Encountered action intent without linkAction` (CHSErrorDomain 1107). The extension needs App Intents metadata (`Metadata.appintents`) also for a system intent. Only Xcode has the tool that makes it (`appintentsmetadataprocessor`). |
| Button action, own intent | Works with metadata written by hand (a test only, user, 2026-11-07). `OpenMenuIntent` in the extension opens `telescreen://menu` with `NSWorkspace`. `Metadata.appintents/extract.actionsdata` has one action, copied from the format of an installed widget (Tailscale); `mangledTypeName` must match the module name (`-module-name TelescreenControl`). chronod: `Successfully ran action`; Control Center: `Button control action succeeded`. The intent runs in the extension (`requiredLaunch: NO`). The sandboxed extension can open the URL: Telescreen was not running, and LaunchServices started it. The format of the metadata is not public, so a release build must not depend on a hand-written file. |
| Refetch | chronod reads the descriptors again only when `CFBundleVersion` changes, and an extension process that still runs answers with the old code. After a change: stop the extension process, increase the version, and run `lsregister` again. |
| Custom SF Symbol | Not tested on this Mac: it needs an asset catalog, and only Xcode has `actool`. `scripts/make-macos-control.sh` calls `actool` with the arguments of swift-build when Xcode is available (CI). In CI (Xcode 26.6), `actool` compiles the symbol into `Assets.car`; it needs the baseline and capline guides of the S, M, and L scales, also when only Regular-M has a drawing. |
| Result | The Control Center button works: an extension with an own intent, the self-signed signature, and an app with a Rust main. A release build needs the Xcode tools for the App Intents metadata and for the asset catalog of the custom glyph (a macOS CI runner has Xcode). The app does not handle `telescreen://menu` yet (step f2). |

### Spike 8, part 2: App Intents metadata in the build (2026-11-07)

User decision: the hand-written metadata is a fallback; CI uses the metadata of the Xcode tool. `scripts/make-macos-control.sh` builds the extension. `TELESCREEN_REQUIRE_XCODE=1` (CI and release) makes the build fail when the tool is missing.

The arguments come from the swift-build sources (swiftlang/swift-build, read 2026-11-07), not from a guess:

- `Sources/SWBApplePlatform/Specs/AppIntentsMetadata.xcspec`: `appintentsmetadataprocessor --toolchain-dir $(TOOLCHAIN_DIR) --module-name $(SWIFT_MODULE_NAME) --sdk-root $(SDKROOT) --xcode-version $(XCODE_PRODUCT_BUILD_VERSION) --platform-family $(PLATFORM_FAMILY_NAME) --deployment-target …`, then the options `--bundle-identifier`, `--output` (the Resources folder of the product), `--target-triple` (one for each architecture), `--binary-file`, `--dependency-file` (the linker `-dependency_info` file), `--stringsdata-file`, `--source-file-list`, `--metadata-file-list`, `--static-metadata-file-list`, `--swift-const-vals-list`, and the default flags `--compile-time-extraction`, `--deployment-aware-processing`, `--no-app-shortcuts-localization`.
- `Sources/SWBApplePlatform/AppIntentsMetadataCompiler.swift`: each file list has one path on each line.
- `Sources/SWBCore/SpecImplementations/Tools/SwiftCompiler.swift`: swiftc gets `-emit-const-values` and `-Xfrontend -const-gather-protocols-file -Xfrontend <file>`, a JSON list of protocol names. The output is a `.swiftconstvalues` file.
- `Sources/SWBApplePlatform/Plugin.swift`: the protocol names (`SWIFT_EMIT_CONST_VALUE_PROTOCOLS`): the App Intents protocols and `AppExtension`, `ExtensionPointDefining`.

Checked with no Xcode: `swiftc -wmo -emit-const-values-path` with the protocol list writes a `.swiftconstvalues` file that names `TelescreenControl.OpenMenuIntent`. The call of `appintentsmetadataprocessor` itself is not tested here (no Xcode); the first CI run of the macOS job tests it. The script compares the `actions` of the generated file with the fallback and warns when they differ.

`actool` for the custom SF Symbol: not done. Read its arguments from swift-build in the same way before the CI step.

## Spike 9: an AppImage of the Linux UI (2026-11-07, AlmaLinux 9, Qt 6.12.0 and 6.8.3, KDE Frameworks 6.20.0, KDE Plasma 6.8)

Question: can one AppImage with its own Qt and Kirigami run on most distributions and follow the KDE color scheme? `packaging/appimage/build.sh --preview` makes it with the QML preview of `telescreen-ui-linux` (no Rust). `.github/workflows/appimage.yml` builds it on AlmaLinux 9 and starts it in other containers.

| Point | Result |
| --- | --- |
| Build | Qt from the official binaries (aqtinstall, `-m qtshadertools qt5compat`; the Wayland client is a default archive). 11 KDE Frameworks from the release tarballs (ECM to qqc2-desktop-style). KArchive needs OpenSSL; Sonnet needs one spell checker backend (Hunspell). About 2 minutes with the cache. |
| Size | Qt 6.12.0: 53.9 MiB AppImage, 176 MiB unpacked. The largest part is the ICU data (30.6 MiB). Qt 6.8.3: 49.4 MiB. |
| glibc | 2.34 with Qt 6.8.3 and with Qt 6.12.0. |
| Other distributions | Starts offscreen in ubuntu:22.04, ubuntu:24.04, debian:12, and fedora:43 with only these host libraries: OpenGL and EGL (libglvnd), fontconfig, freetype, a font, libxkbcommon, libdbus, glib (no recommended or weak packages). |
| KDE Plasma 6.8, Breeze Dark | The window follows the color scheme, the accent color, the Breeze icons, and the fonts of the host, with no plasma-integration in the AppImage. The direct start (FUSE) works. Difference: the buttons show the mnemonic underlines; the KDE platform theme normally hides them. |
| linuxdeploy-plugin-qt | It does not deploy `plugins/wayland-graphics-integration-client`: with no EGL integration, Qt Quick has no OpenGL context on Wayland. It does not find the KDE style, because Kirigami selects it at runtime. Copied plugins need `--deploy-deps-only` for each file (not recursive); else they load the KDE Frameworks of the host, which need the Qt of the host (6.12), and fail. |
| Qt Shape | The curve renderer of Qt 6.8.3 keeps a stale outline after a resize (wrong card sizes). Qt 6.12.0 is correct. Thus the AppImage has Qt 6.12. |
| Result | Feasible for the release (user, 2026-11-07: a new Qt in the AppImage). Not tested: the `telescreen` binary with the Qt UI (the app does not link Qt yet), the applet install from the AppImage, GNOME and niri. |

## Spike 10: a virtual touchpad and the trackpad fingers of macOS (2026-11-08, KWin 6.8.2, libinput 1.32.0, Chrome on Wayland, macOS 27)

Question: can a uinput touchpad on Linux replace the button scroll of spike 5, so that a scroll in Chromium does not stay on one pane? The code is not kept. The test code made a uinput multitouch touchpad, and then forwarded the trackpad fingers from the portal of the Mac.

| Question | Result |
| --- | --- |
| Device | libinput takes a uinput device with `ABS_MT_SLOT`, `ABS_MT_TRACKING_ID`, `ABS_MT_POSITION_X/Y` with a resolution, `BTN_TOUCH`, `BTN_TOOL_FINGER` to `BTN_TOOL_QUINTTAP`, and `INPUT_PROP_POINTER` as a touchpad. Do not set `INPUT_PROP_BUTTONPAD`: a finger that starts in its software button area does not scroll. `BUS_VIRTUAL` keeps the device from the KDE setting that turns off a touchpad when a mouse is present. |
| Two panes in Chromium | Each two-finger gesture scrolls the pane below the pointer: the lift of the fingers ends the scroll. |
| libinput scroll | No acceleration: Chrome scrolls about 105 CSS pixels for each mm. libinput does not scroll the first 0.7 mm. A short contact with little motion is a two-finger tap (a right click); 250 ms of contact gave no tap. |
| KWin settings of the device | `org.kde.KWin.InputDevice` on D-Bus: `scrollFactor` and `naturalScroll` are writable, and KWin stores them in `kcminputrc` for the name, vendor, and product of the device only. |
| Scroll deltas of macOS to finger motion | Fails. The point deltas of macOS have acceleration, and most of the distance is in the momentum. A normal scroll moved the fingers less than 1 mm: the scroll needed a large motion and often was a right click. |
| Trackpad fingers on macOS | `NSTouch` (public AppKit API): a view sets `allowedTouchTypes` to the indirect mask (`1 << 1`; the value 1 is direct touches) and gets `touchesBegan/Moved/Ended/Cancelled`. Each touch has an identity, a normalized position (0 to 1, from the bottom-left), and `deviceSize` in points (this trackpad: 343 x 210, about 121 mm x 74 mm). Updates come about each 16 ms. AppKit gives them only to the key window under the cursor. One change of two fingers comes as two events, about 0.1 ms apart, each with one moved finger. |
| Event tap and touches | An `NSEvent` made from a gesture event (type 29) in the event tap has no touches. Gesture field 110: 0 is a plain touch event, 6 a scroll, 23 the Dock swipe between Spaces. The touches reach the view only if the tap passes the events with field 110 = 0. |
| Fingers forwarded 1:1 in mm | It worked, but the user found it much worse than the button scroll of spike 5: too fast also with a KWin scroll factor of 0.2, not smooth (network jitter on the reliable channel, p99 71 ms that day), and the feel is that of a Linux touchpad, not of the Mac. Three fingers drag on the Mac, so Linux gestures with three fingers conflict. |
| Result | Not used (user, 2026-11-08). The button scroll stays (IN-15). |
