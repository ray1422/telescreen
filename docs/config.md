# Configuration v1 (Design Draft)

This document and `schema/config.schema.json` together define the contract. At this time, there is no application implementation and no driver implementation. YAML is applicable to the nested computer → device → capability config. JSON Schema Draft 2020-12 validates the parsed data. The defaults in the schema are hints. The executor must still actually apply them.

## Structure and Identity

- `version: 1`: the config format version. Reject unknown versions at load time.
- `computers`: the keys are stable computer IDs. Specify the local ID explicitly at startup. You must not guess it from the hostname.
- `endpoint: {host, port}`: the connection address of the agent. It does not represent a trusted identity. Keep the pairing identity in local runtime data. Do not write secrets to this file.
- `devices`: a mapping. Its keys are local aliases. The aliases have no OS-identification semantics at all.
- `id`: the shared physical device ID. An `id` must not be duplicated in the same computer. Different computers use the same ID to show that they connect to the same physical device. Only the listed connections exist.
- `label`: optional display text.
- `portal-match`: an optional condition operation. It decides whether the device applies to the current portal. The core does not specify display names, serials, EDID, or an identification algorithm.
- `input-target`: an optional resolution operation. On the receiving computer, it selects the local area where the cursor is active. It is not possible to get coordinates from the shared device ID alone. You can still hand off a device that does not have this capability. But portal mouse forwarding is not available for that device.
- `handoff.<target-computer-id>`: the method that this computer uses to switch the device to the specified computer. `steps` run in order. Execution stops immediately at the first error. The optional `verify` checks whether the switch actually succeeded.

The config defines no required fields such as `portals`, `kind`, `source`, or `match.name`. Video input values such as dp and hdmi are private parameters of the driver. The UI creates windows and selects targets at runtime. Input focus and device ownership are different states. A device can be a display, a USB switch, or another switchable resource.

All operations run on the computer that they belong to. Do not accept arbitrary shell commands from a remote computer. The local device that starts the operation sets the routing. The computer that the display shows at this time has no effect on the routing. Even while the display shows PC1, the Mac can still run its own handoff.pc2. The driver must be able to select the target from any supported current state. If the device does not allow control from a video input other than the displayed one, the driver should fail explicitly. v1 does not automatically give the execution to another computer.

## Operations and Extensions

```yaml
portal-match:
  driver: command
  timeout-ms: 5000
  options:
    argv: [/absolute/path/helper, portal-match]
```

The built-in design baseline has only `command`. Other drivers must use a namespace, for example `lab.http`. That driver validates its own `options`. The JSON Schema permits extensions, but this does not mean that the driver is installed or that HTTP is supported. The local capability check must report an unknown driver as unsupported. The check must not silently treat an unknown driver as success. Check only whether local operations can run. The Mac does not need to have Linux helpers installed.

`command.options` supports `argv`, an optional `cwd`, and an optional string map `env`. Do not do shell expansion, string templating, `~` expansion, or implicit shell execution. The executable and cwd must be absolute paths for that platform (semantic validation). If cwd is not specified, use the directory that contains the config file. env is a layer on top of the environment of the current agent. If a shell is necessary, you must specify it explicitly in argv. Thus, the config file is trusted executable configuration.

The default timeout is 5000 ms. The maximum is 300000 ms. Each operation has its own timer. On timeout, you must stop the child process tree. The limit for stdout is 64 KiB. The limit for stderr is also 64 KiB. You must not start helpers from keyboard hooks or on each mouse move. Run helpers only at these times: during candidate probing, when the display topology or the window position changes, and before a takeover. Coalesce consecutive events.

## Driver I/O Contract (protocol 1)

The stdin of the command is one UTF-8 JSON object. stdin is closed after the object is written. stdout is one JSON result. Diagnostics go to stderr. Exit code 0 means that the protocol completed. Any non-zero exit code is always an error, also exit 1. The exit code never shows that a condition is false. This rule replaces the 0/1 boolean draft from an earlier discussion.

Fixed request fields:

```json
{
  "protocol": 1,
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

`operation` is `portal-match`, `input-target`, `handoff`, or `verify`. `computer_id` is always the executing computer. source/target give the direction of this connection. target is null when there is no target. portal is null for operations that are not portal operations, and for a remote input-target. session_id is present only after a session is established. A display token is an opaque ID from the current OS display enumeration. It is not valid on a different computer or after a restart.

bounds use local desktop logical coordinates that the core normalizes. The origin is at the top-left. x increases to the right. y increases downward. Negative coordinates are permitted. The backend is responsible for the OS coordinate conversion. scale is the ratio of logical pixels to physical pixels. metadata is optional information. It is platform-dependent and extensible. You must not assume that name/serial is present. If a platform cannot get global coordinates, it returns portal.bounds = null and display.bounds = null. portal.display_token can also be null. The driver can use the token/metadata that is available, or it can explicitly report unsupported. You must not make up coordinates. This I/O contract is a text specification. The config schema does not validate runtime payloads.

Each purpose of the call has a fixed result:

| operation | stdout on success | Semantics |
| --- | --- | --- |
| portal-match | `{"matched":true}` or `{"matched":false}` | false is a normal result with no match |
| input-target | `{"display_token":"display-session-2"}` | Selects one valid token from the displays of the receiving computer. The core backend is responsible for the mapping |
| handoff | `{"ok":true}` | The driver states that the step is complete |
| verify | `{"matched":true}` or `{"matched":false}` | false means that the switch is not confirmed. Stop, and do not enable forwarding |

A non-zero exit code, invalid JSON, a wrong result shape, a stale display token, and a timeout are all errors. Native/extension drivers obey the same logical contract. But they do not have to actually start a process. In v1, input-target is limited to a full display. Arbitrary sub-regions and other pointer surfaces are for future protocol versions.

## Runtime Resolution and Handoff

1. Evaluate only the local devices that have portal-match. Show 0 matches as unpaired. Multiple matches need a selection. Bind this selection to the current portal session. Do not simply use the first match. A window move or a topology change makes a new evaluation necessary.
2. The targets are the intersection of the local handoff keys and "other computers that have the same id". A route in one direction does not mean that the opposite direction is available. Offline targets can be shown. A switch to an offline target is not possible.
3. After the selection, do these steps for the receiving computer. Find its local device by the shared id. Resolve input-target. Confirm the input-injection capability and authorization. Get an exclusive session lease on that device. Input focus belongs to only one portal at a time.
4. Pause input forwarding. Release the held state of the previous session. Then run steps on the initiating computer. Then run verify, if it is present. Then the receiving computer shows the confirmation prompt, and the user confirms with a key on the source computer (`docs/adr/0002-user-confirms-handoff.md`). Enable forwarding only after this user confirmation and after you confirm that the display topology of the target is still valid. Do not forward the click on the switch button to the remote computer.
5. Without verify, success means only that the driver reported completion. It does not guarantee that the video input actually changed. The UI must show "confirmed" and "unverified" as different states. The user confirmation of step 4 also confirms the video input. On failure, keep local control. Do not do a blind retry. Do not claim a rollback. After a partial success, it is possible that the physical state is unknown.
6. An end or a disconnect releases input and the lease. Handoff does not automatically run in reverse. A switch of the video input back needs an explicit route. The UI can also start it from another local window.

You can manually switch devices without portal-match, such as USB, from the device list. They do not automatically follow a display switch. Multi-device transactions, remote execution relays (jump hosts), and a condition DSL are all outside the scope of v1.

## Validation Layers

1. Parsing: accept only the JSON-compatible subset of YAML. These items are prohibited: duplicate keys, custom tags, merge keys, aliases, non-string keys, non-finite numbers, and non-JSON types.
2. JSON Schema: types, required fields, unknown fields, the operation envelope, timeouts. Reject unrecognized fields, except in extension options, to find typos.
3. Semantic: ids are unique in the same computer. The target computer exists and is not the computer itself. The target has the same id. command paths are absolute for each platform.
4. Capability check: whether the driver is available, whether options are valid, whether helpers exist, whether permissions/remote are ready. If the config passes the schema, this does not mean that the hardware is usable.

`tools/validate_config.py` implements offline checks for the first 3 layers. It accepts absolute platform paths in POSIX format or Windows format. The computer that actually runs the operation must still validate paths and capabilities again for the local platform. The example intentionally uses helpers that do not exist yet and example drivers. The validator does not run them.
