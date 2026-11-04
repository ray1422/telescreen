# Configuration v1

This document and `schema/config.schema.json` together define the contract. The config `version` stays 1: the format changed for v0.3.0, but there was no release, so there is no migration (user, 2026-10-27). YAML is applicable to the nested computer → device → capability config. JSON Schema Draft 2020-12 validates the parsed data. The defaults in the schema are hints. The executor must still actually apply them.

## Location

The default file is `$XDG_CONFIG_HOME/telescreen/config.yaml`, else `~/.config/telescreen/config.yaml`, also on macOS (spec PEER-5). `--config` replaces it.

## Structure and Identity

- `version: 1`: the config format version. Reject unknown versions at load time.
- `computers`: the keys are stable computer IDs. The user selects the local ID (spec PEER-3). You must not guess it from the hostname.
- `hint`: optional plain text, 1 to 500 characters. The `prompt` verifier of this computer shows it after a handoff to this computer (spec VER-5). Example: the copy/paste shortcuts of this computer. It is not a template and not markup.
- `clipboard`: optional. `clipboard.text` (boolean, default `true`) is the start value of the text clipboard switch of this computer (CLIP-1). The control window can change the switch while the app runs; the change is not saved. No other key is permitted in `clipboard`.
- `scroll-speed`: optional number, larger than 0 and not larger than 10 (default `1`). This computer multiplies each scroll that it receives from a source by this number (spec IN-14). Example: `0.4` on the Linux computer makes the scroll from the Mac slower.
- `disabled-quirks`: optional list of workaround names, with no duplicates (default: an empty list). This computer uses each platform workaround, unless this list has its name (spec IN-16). An unknown name is an error. The change takes effect when the app starts. The names:
  - `kwin-button-scroll`: on KWin, a forwarded scroll is a button scroll (spec IN-15). If this list has the name, a forwarded scroll on KWin is a wheel scroll, and applications can animate it.
- `path`: optional list of absolute directories. Telescreen finds a program name with no `/` in these directories, in this sequence (see "Programs and the search path"). If `path` is absent, Telescreen uses the `PATH` of the login shell of the user.
- `endpoint: {host, port, discovery}`: the connection address of the agent. It does not represent a trusted identity. Keep the pairing identity in local runtime data. Do not write secrets to this file. `host` is one IP address or DNS name, or a list of them. Telescreen resolves each host at each dial and uses its first IPv4 address (spec NET-11). `port` is the UDP port where this computer listens. `discovery: mdns` lets the other computers find this computer with the mDNS responder of the system (spec NET-12 to NET-16). `host` is optional only with `discovery`. A connection from a different address is accepted if its fingerprint is trusted (spec NET-3).
- `devices`: a mapping. Its keys are local aliases. The aliases have no OS-identification semantics at all.
- `id`: the shared physical device ID. An `id` must not be duplicated in the same computer. Different computers use the same ID to show that they connect to the same physical device. Only the listed connections exist.
- `label`: optional display text.
- `portal-match`: an optional condition operation. It decides whether the device applies to the current portal. The core does not specify display names, serials, EDID, or an identification algorithm.
- `input-target`: an optional resolution operation. On the receiving computer, it selects the local area where the cursor is active. It is not possible to get coordinates from the shared device ID alone. You can still hand off a device that does not have this capability. But portal mouse forwarding is not available for that device.
- `handoff.<target-computer-id>`: the method that this computer uses to switch the device to the specified computer. `steps` run in order. Execution stops immediately at the first error. A route has no `verify`.
- `verify`: an optional verification operation. This computer runs it when the device switches to this computer, after the steps of the other computer (spec VER-1, ADR 0009). It is the same for a handoff and a hand back. No `verify` means no check: the device state is "unverified" (spec VER-2).

The config defines no required fields such as `portals`, `kind`, `source`, or `match.name`. Video input values such as dp and hdmi are private parameters of the driver. The UI creates windows and selects targets at runtime. Input focus and device ownership are different states. A device can be a display, a USB switch, or another switchable resource.

All operations run on the computer that they belong to. Do not accept arbitrary shell commands from a remote computer. The local device that starts the operation sets the routing. The computer that the display shows at this time has no effect on the routing. Even while the display shows PC1, the Mac can still run its own handoff.pc2. The driver must be able to select the target from any supported current state. If the device does not allow control from a video input other than the displayed one, the driver should fail explicitly.

There is one exception to "no execution for another computer" (ADR 0010): the source of an active, authenticated session can request a hand back. The target then runs its own `handoff.<source>` route. The request names no command and no route, so it can only cause the route back to the peer that sent it.

## Operations and Drivers

An operation is a mapping with a `driver`. The driver defines how Telescreen reads the result of a program. It does not define which program runs: the user writes the whole command, also the program (user, 2026-10-27).

```yaml
portal-match:
  driver: exec
  timeout-ms: 5000
  command: [lg-helper, portal-match]
verify:
  driver: prompt
```

Fields:

- `driver`: required. A built-in driver (`exec`, `command`, `prompt`) or a namespaced extension driver.
- `command`: required for `exec` and `command`. A string or a list, as in Docker Compose. Telescreen splits a string into arguments by POSIX shell rules (quotes and backslashes), but no shell runs it: there is no variable expansion, no glob, no `~` expansion, no pipe, and no `&&`. A list is used as it is. For shell features, write the shell explicitly, for example `[/bin/sh, -c, "a | b"]`. Thus the config is trusted executable configuration.
- `cwd`: optional absolute directory. The default is the directory that contains the config file.
- `env`: optional string map. It is a layer on top of the environment of the app.
- `timeout-ms`: optional, 1 to 300000. The default is 5000, and 60000 for `verify`, because a verifier can wait for the user.

Built-in drivers:

| driver | Operations | Result |
| --- | --- | --- |
| `exec` | `portal-match`, handoff steps, `verify` | Only the exit code. 0 is success or true. A non-zero exit is failure or false: `portal-match` does not match, a step fails, `verify` is not ok. stdin is empty, and stdout is not read. A program that cannot start is an error |
| `command` | all | Protocol 2 (below): JSON on stdin and stdout. Each non-zero exit is an error |
| `prompt` | `verify` only | The built-in verifier (spec VER-5). It has no other field |

Drivers for the output format of one tool (for example `betterDisplay` or `ddcutil`, with `expect`) come in a later version (user, 2026-10-27).

Other drivers must use a namespace, for example `lab.http`, and keep their parameters in `options`. That driver validates its own `options`. The JSON Schema permits extensions, but this does not mean that the driver is installed or that HTTP is supported. The local capability check must report an unknown driver as unsupported. The check must not silently treat an unknown driver as success. Check only whether local operations can run. The Mac does not need to have Linux helpers installed.

Each operation has its own timer. On timeout, you must stop the child process tree. The limit for stdout is 64 KiB. The limit for stderr is also 64 KiB. You must not start helpers from keyboard hooks or on each mouse move. Run helpers only at these times: during candidate probing, when the display topology or the window position changes, before a takeover, and after a handoff to this computer (`verify`). Coalesce consecutive events.

### Programs and the search path

- A program with a `/` must be an absolute path. Telescreen does not find it in the search path.
- A program name with no `/` is found in the search path (user, 2026-10-27). If the computer section has `path`, use it. Else read `PATH` from the login shell of the user one time at the start: `$SHELL -l -c` with a command that prints `PATH`. Reason: an app that macOS starts from Finder or `open` has only the system directories in its own `PATH`.
- The program gets the search path as its `PATH`, so that a script finds the same programs. `env` can replace it (Claude decides, 2026-10-27).
- If Telescreen cannot read the `PATH` of the login shell, it uses its own `PATH` and shows a notice in the control window (Claude decides, 2026-10-27).
- `open` on macOS starts an app through Launch Services and returns at once. The output of the app does not come back, and the exit code is the exit code of `open`. Use the executable in the app bundle, or `open --stdout`.

## The `command` Driver (protocol 2)

For `portal-match`, `input-target`, and handoff steps, the stdin of the command is one UTF-8 JSON object. stdin is closed after the object is written. stdout is one JSON result. `verify` uses JSON lines in both directions (below). Diagnostics go to stderr. Exit code 0 means that the protocol completed. Any non-zero exit code is always an error, also exit 1. The exit code never shows that a condition is false: for that, use `exec`.

Protocol 2 replaces protocol 1 (user, 2026-10-27). The single-object operations did not change; protocol 2 adds the JSON lines of `verify`.

Fixed request fields:

```json
{
  "protocol": 2,
  "operation": "portal-match",
  "computer_id": "mac",
  "device": {"id": "shared-lg", "alias": "Desktop"},
  "source_computer_id": "mac",
  "target_computer_id": null,
  "session_id": null,
  "portal": {
    "id": "window-1",
    "bounds": {"x": 1920, "y": 0, "width": 3440, "height": 1440},
    "coordinate_space": "local-desktop-logical",
    "display_token": "display-session-2"
  },
  "displays": [
    {
      "token": "display-session-2",
      "bounds": {"x": 1920, "y": 0, "width": 3440, "height": 1440},
      "scale": 1.0,
      "metadata": {"name": "Desktop"}
    }
  ]
}
```

`operation` is `portal-match`, `input-target`, `handoff`, or `verify`. `computer_id` is always the executing computer. source/target give the direction of this connection. target is null when there is no target. For `verify`, `computer_id` and `target_computer_id` are the destination, and `source_computer_id` is the computer that ran the steps. portal is null for operations that are not portal operations, and for a remote input-target. session_id is present only after a session is established. It is null for the `verify` of a hand back, because the session already ended. A display token is an opaque ID from the current OS display enumeration. It is not valid on a different computer or after a restart.

bounds use local desktop logical coordinates that the core normalizes. The origin is at the top-left. x increases to the right. y increases downward. Negative coordinates are permitted. The backend is responsible for the OS coordinate conversion. scale is the ratio of logical pixels to physical pixels. metadata is optional information. It is platform-dependent and extensible. You must not assume that name/serial is present. If a platform cannot get global coordinates, it returns portal.bounds = null and display.bounds = null. portal.display_token can also be null. The driver can use the token/metadata that is available, or it can explicitly report unsupported. You must not make up coordinates. This I/O contract is a text specification. The config schema does not validate runtime payloads.

Each single-object operation has a fixed result:

| operation | stdout on success | Semantics |
| --- | --- | --- |
| portal-match | `{"matched":true}` or `{"matched":false}` | false is a normal result with no match |
| input-target | `{"display_token":"display-session-2"}` | Selects one valid token from the displays of the receiving computer. The core backend is responsible for the mapping |
| handoff | `{"ok":true}` | The driver states that the step is complete |

A non-zero exit code, invalid JSON, a wrong result shape, a stale display token, and a timeout are all errors. Native/extension drivers obey the same logical contract. But they do not have to actually start a process. In v1, input-target is limited to a full display. Arbitrary sub-regions and other pointer surfaces are for future protocol versions.

### `verify` in JSON lines

Each line is one UTF-8 JSON object and ends with `\n`. The verifier talks with the core (spec VER-3):

1. The core writes the request object (above, with `"operation": "verify"`) as the first line on stdin. stdin stays open.
2. The verifier writes lines on stdout:
   - `{"show":"<text>"}`: the core shows the plain text (1 to 500 characters) in the verifier window on the switched display. A new `show` replaces the text.
   - `{"want":"ack"}`: the verifier asks for an acknowledgement of the user. The core decides which user actions acknowledge (spec VER-4). When the user acknowledges, the core writes `{"ack":true}` on stdin.
   - `{"ok":true}` or `{"ok":false}`: the result. It is the last line. Then the verifier exits with 0.
3. The core closes stdin after the result, or when it stops the verifier.

The window opens at the first `show` or `want`. With no `show`, it shows only how to acknowledge (Claude decides, 2026-10-27). The core sends one `ack` for each `want`, after the user acknowledges. `show` and `want` need an `input-target` of the device on the destination, because the window shows on that display. Without it, they are errors. Each other line, a missing result, a non-zero exit, and a timeout are errors. An error, `{"ok":false}`, and a timeout all make the device state unknown (spec VER-6). The protocol has no other user input (spec VER-7).

The built-in `prompt` verifier does the same as this verifier: `{"show":"<hint>"}`, `{"want":"ack"}`, then `{"ok":true}` after the acknowledgement, or `{"ok":false}` after 60 seconds.

## Runtime Resolution and Handoff

1. Evaluate only the local devices that have portal-match. Show 0 matches as unpaired. Multiple matches need a selection. Bind this selection to the current portal session. Do not simply use the first match. A window move or a topology change makes a new evaluation necessary.
2. The targets are the intersection of the local handoff keys and "other computers that have the same id". A route in one direction does not mean that the opposite direction is available. Offline targets can be shown. A switch to an offline target is not possible.
3. After the selection, do these steps for the receiving computer. Find its local device by the shared id. Resolve input-target. Confirm the input-injection capability and authorization. Get an exclusive session lease on that device. Input focus belongs to only one portal at a time.
4. Pause input forwarding. Release the held state of the previous session. Then run the steps on the initiating computer. Then the destination runs the `verify` of its device (spec VER-1). Enable forwarding only after the verifier answers ok, or after the steps if the destination has no `verify`, and only after you confirm that the display topology of the target is still valid. Do not forward the click on the switch button to the remote computer.
5. Without `verify`, success means only that the steps completed. It does not guarantee that the video input actually changed. The UI must show "confirmed" and "unverified" as different states (spec VER-6). On failure, keep local control. Do not do a blind retry. Do not claim a rollback. After a partial success, the device state is unknown.
6. An end or a disconnect releases input and the lease. Only a hand back runs a route back (spec HB-1): the target runs its own `handoff.<source>` route, then the source runs its own `verify`.

You can manually switch devices without portal-match, such as USB, from the device list. They do not automatically follow a display switch. Multi-device transactions, remote execution relays (jump hosts), and a condition DSL are all outside the scope of v1.

## Validation Layers

1. Parsing: accept only the JSON-compatible subset of YAML. These items are prohibited: duplicate keys, custom tags, merge keys, aliases, non-string keys, non-finite numbers, and non-JSON types. Also prohibited: more than one document, anchors, and explicit tags (also `!!str`). Plain scalars resolve by the YAML 1.2 core schema. A plain (unquoted) scalar that YAML 1.1 resolves to a different type is ambiguous, and the parser rejects it (user, 2026-10-27). Quote it to get a string. The ambiguous forms:
   - YAML 1.1 booleans: `y`, `Y`, `n`, `N`, and `yes`, `no`, `on`, `off` in lower case, title case (`Yes`), or upper case (`YES`).
   - Timestamps: a scalar that starts with `YYYY-M-D` (four digits, a hyphen, one or two digits, a hyphen, one or two digits).
   - Integers with a leading zero (`010`), with an optional sign.
   - Binary integers (`0b101`) and `0o` octal integers (`0o17`), with an optional sign. YAML 1.1 has no `0o` form, so PyYAML reads `0o17` as a string.
   - Numbers with `_`: an optional sign, then a digit, then digits, `.`, and `_`, with one `_` or more (`1_000`).
   - Sexagesimal numbers: an optional sign, a digit, digits or `_`, one or more groups of `:` and digits or `_`, then an optional `.` and digits or `_` (`1:30`, `190:20:30.5`).
   - Numbers with an exponent: an optional sign, digits with an optional `.` (or a `.` and digits), `e` or `E`, an optional sign, digits (`1e3`, `1.5e3`, `1.5e+3`, `.5e3`). PyYAML reads an exponent with no sign as a string.
   - `=` alone. PyYAML reads it as a special value type.
   - `.inf`, `.nan`, and their case forms. They are also not finite.
2. JSON Schema: types, required fields, unknown fields, the operation envelope, timeouts. Reject unrecognized fields, except in extension options, to find typos.
3. Semantic: ids are unique in the same computer. The target computer exists and is not the computer itself. The target has the same id. A `command` string splits into one argument or more. A program with a `/`, `cwd`, and each `path` entry are absolute for each platform.
4. Capability check: whether the driver is available, whether options are valid, whether helpers exist, whether permissions/remote are ready. If the config passes the schema, this does not mean that the hardware is usable.

`tools/validate_config.py` implements offline checks for the first 3 layers. It accepts absolute platform paths in POSIX format or Windows format. The computer that actually runs the operation must still validate paths and capabilities again for the local platform. The example intentionally uses helpers that do not exist yet and example drivers. The validator does not run them.
