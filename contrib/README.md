# Contributed helpers

Telescreen does not identify displays or switch video inputs itself. Drivers do it (`docs/config.md`). These files help you write a config for the `command` driver (protocol 2).

## display-helper

A protocol 2 helper in Python 3 (standard library only). Copy it to a directory in the search path (`docs/config.md`), for example `/usr/local/bin/telescreen-display-helper`, and make it executable.

- `display-helper match KEY=VALUE...`: for `portal-match`. The result is true if the portal display has all the metadata values.
- `display-helper target KEY=VALUE...`: for `input-target`. It selects the one display that has all the metadata values. No match or more than one match is an error.
- `display-helper run -- PROGRAM ARG...`: for a `handoff` step with the `command` driver. It runs the program with no shell. Exit status 0 gives `{"ok": true}`; each other status is an error. The `exec` driver does the same with no helper (`example-lg.yaml` uses it).
- `display-helper show`: writes the `displays` of a request to stderr, to find the metadata values. Telescreen logs stderr of a failed helper.

Metadata (see `crates/telescreen-platform-*/src/displays.rs`):

| Platform | Keys |
| --- | --- |
| macOS | `vendor`, `model`, `serial`, `builtin`, `name` |
| Linux (Wayland) | `name` (the output name, for example `HDMI-A-2`), `description`, `make`, `model` |

## Example: one LG display, a Mac, and a Linux computer

`example-lg.yaml` is a config for this setup. The display switch commands are specific to some LG displays. Change them for your display.

- macOS uses BetterDisplay: `BetterDisplay set -namelike=<name> -ddcAlt=<value> -vcp=inputSelectAlt`.
- The example uses `.local` host names. They need a system resolver for `.local` names, or use `endpoint.discovery: mdns` (`docs/config.md`). If mDNS does not resolve them on your network (`getent hosts <name>` on Linux, `dscacheutil -q host -a name <name>` on macOS), use IP addresses.
- Linux uses ddcutil. The ddcutil command in the example is not tested (see `docs/hardware.md`).
