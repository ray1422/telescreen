---
status: superseded
superseded-by: 0008
---

# On macOS, a disclaimed helper process does the network I/O

Superseded by ADR 0008 (2026-10-27): the helper was removed. The premise of this ADR was wrong; ADR 0008 gives the reasons.

macOS Local Network privacy blames a connection on its responsible process. An ad-hoc signed app installed from a dmg did not get the Local Network prompt and failed with "No route to host" (`docs/spikes.md`, spike 4). Command-line tools are not subject to this check. Thus `Telescreen.app` does the UI, the event tap, and the injection, and it holds the Accessibility and Input Monitoring permissions. It starts `telescreen-net` with `posix_spawn`, `POSIX_SPAWN_SETEXEC`, and the private `responsibility_spawnattrs_setdisclaim` (weak-linked), so that `telescreen-net` is its own responsible process. `telescreen-net` does all QUIC I/O. The two processes talk over a Unix domain socket in the user's runtime directory. A local socket is not subject to Local Network privacy.

## Implementation (2026-10-27)

The helper is not a separate binary. It is the hidden subcommand `telescreen net-helper` of the same executable, so the bundle has one binary and one signature. The app makes a Unix domain socket in a new private directory (mode 0700), starts the helper with the disclaim attribute, and waits for its connection. Each `NetCommand` and `NetEvent` is one JSON line. When the helper stops, the app ends all sessions (AC-18). If the helper cannot start, the network runs in the app, and the UI shows a notice. Not tested yet: if the disclaim works for a binary inside the app bundle (AC-25).

## Consequences

- One more local hop for each input event. Its cost is small compared to the Wi-Fi round trip (spike 4). Measure it in the product.
- If macOS drops `responsibility_spawnattrs_setdisclaim`, the helper still starts, without the disclaim. Then the Local Network permission applies to the app again. Show this state in the UI.
- The app still has `NSLocalNetworkUsageDescription` in `Info.plist`.
- Linux does not need the helper. The same transport code runs in the main process, behind the same interface.
