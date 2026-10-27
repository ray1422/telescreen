# Debug protocol (dev builds only)

The debug protocol lets a local tool read the state of the app and send intents, so that a hardware test can run without clicks (user, 2026-10-27). It is not part of a release.

## Build and start

- The protocol is behind the Cargo feature `debug-protocol` of `telescreen-app`. The feature is off by default. A release build must not enable it: the protocol can start a handoff and confirm a session.
- Build with `cargo build --release -p telescreen-app --features debug-protocol`. `scripts/make-macos-app.sh` enables it when `TELESCREEN_DEBUG=1`.
- Start with `telescreen run --config <config> --computer <id> --debug-socket <path>`. Without the feature, `--debug-socket` is an unknown argument, and the app does not start.
- The socket is a Unix domain socket with mode 0600. If a socket file is at the path, the app replaces it. If a different file is at the path, the app does not start.

## Messages

Each request and each response is one line of JSON. One connection can send more than one request. The responses come in the sequence of the requests. The app answers each request on its main thread. Requests do not go through the input callbacks (IN-2).

| Request | Effect | Response |
| --- | --- | --- |
| `{"cmd":"state"}` | none | `{"ok":true,"control":<text>,"portal":<text or null>,"windows":[<window>...]}` |
| `{"cmd":"intent","intent":<intent>}` | the same as the click in the UI | `{"ok":true}` |
| `{"cmd":"confirm"}` | the same as the confirmation key in the focused portal (FLOW-8); the focus check applies (AC-6) | `{"ok":true}`, or an error if the portal does not wait for a confirmation |
| `{"cmd":"quit"}` | the same as closing the control window | `{"ok":true}` |
| `{"cmd":"hold_keyboard","display":<display token>}` | calls the focus holder of this peer directly (UI-7), with no session | `{"ok":true}`, or an error if the peer has no focus holder or no such display |
| `{"cmd":"release_keyboard"}` | removes the focus holder | `{"ok":true}` |

An invalid request gets `{"ok":false,"error":<text>}`.

`control` and `portal` are the Rust `Debug` text of the view models. They are for people, not for parsers.

A window is `{"role":"control"|"portal"|"prompt","monitor":<display token or null>,"fullscreen":<bool>,"focused":<bool>,"position":[x,y] or null,"size":[w,h]}`. Position and size are logical pixels. Wayland does not report the position.

An intent is one of: `"OpenPortal"`, `"ClosePortal"`, `"EndSession"`, `{"StartHandoff":{"target":<computer id>}}`, `{"SelectDevice":{"alias":<device alias>}}`, `{"StopSession":{"session":<id>}}`, `{"ControlFromHere":{"session":<id>}}`, `{"SetClipboard":{"on":<bool>}}`.

Example with `socat`:

```sh
echo '{"cmd":"state"}' | socat - UNIX-CONNECT:/tmp/telescreen.sock
```

## Limits

- The protocol does not inject input and does not replace the input capture. The hardware tests of the capture (AC-14 to AC-17) still need the user.
- `StartHandoff` runs the handoff steps of the config. With the real config, it switches the shared display. For a test without a switch, use a config whose steps only print `{"ok":true}`.
- `confirm` on the source arms the input capture and suppresses the local input.
- With a clipboard switch that is on, a session changes the clipboard of the other computer (CLIP-2). Send `SetClipboard` with `on` false on both computers before a test that must not change the clipboard.
- `hold_keyboard` takes the keyboard focus from all applications of this computer until `release_keyboard`, the session end, or the app end. Use it only on a computer where nobody types. The `focused` facts of the windows show the effect.
