# Telescreen Implementation Handoff

Status: 2026-10-27. The requirements and the schema design are delivered. The application is not implemented yet. The person who continues this work should first read this document, `docs/config.md`, `examples/config.yaml`, and `schema/config.schema.json`. `docs/architecture.md` gives the sources for the platform APIs and the technical trade-offs.

## What the user wants to do

Share a keyboard and a mouse in an environment that has physical display connections. The typical environment is a MacBook with a built-in display and an external display. PC1 and PC2 also connect to this external display. The keyboard and the mouse stay on the Mac. The target computer sends the picture through a physical video cable. Thus, v1 does not need to send remote desktop video.

The user opens a portal window on the Mac. The user moves the window to the external display and makes it fullscreen. The window shows a list of the applicable computers. The user clicks "Switch to PC1". Then a custom action runs and switches the device, so that the physical display shows PC1. The Mac captures and suppresses the keyboard and mouse events for the corresponding region. The Mac forwards these events to PC1. PC1 injects them as local events and controls the cursor. While the cursor is in the region of the portal, input focus follows the cursor. The user does not have to click before the user can type.

You must not hardcode Mac, Linux, two computers, or a single display. There can be more than one computer and more than one display. It is possible that only some of the computers connect to the same device. Devices are not only displays. The config can also describe any equipment that configured operations can switch.

## Confirmed design direction

- Use YAML. Nest it as computer → devices, so that the structure is not fully flat.
- Do not configure portal instances. Create windows at runtime. Each window selects its target.
- Local device aliases can be different, for example Desktop on the Mac and Monitor1 on Linux. A shared `id` maps them to the same physical device.
- Aliases are not for OS matching. The core does not hardcode identification methods that use the name, EDID, serial, or video input.
- Drivers implement abstract conditions and operations. command is only one type of driver. Each computer can define a different handoff.
- List only the devices that are actually connected. A handoff has a direction. Do not assume that a handoff is symmetric in both directions.
- When the mouse moves to the top edge, the macOS window buttons/control bar can appear and cover the interface. Fullscreen should prevent this as much as possible.
- Shortcut suppress/forward and remote cursor control are required. Clipboard access and clipboard sharing are desired features.

## Schema decisions after this round of consolidation

`docs/config.md` is the normative specification. The list that follows is a summary:

- Top level: `version`, `computers`. Each computer has `endpoint` and `devices`.
- A device must have a shared `id`. `portal-match`, `input-target`, and `handoff` are optional.
- `portal-match` is an abstract condition. The core does not hardcode an identification method.
- `input-target` resolves the current display token on the receiving computer. This is for the problem that "a shared device identity is not the same as remote coordinates". `input-target` does not require a specific identification method.
- `handoff.<target>` has `steps` and an optional `verify`. `steps` are operations that run in order.
- An operation uses `driver`, `options`, and an optional `timeout-ms`. command uses argv. It does not use string templating or an implicit shell.
- command uses JSON on stdin/stdout. Any non-zero exit is an error. The JSON gives the true/false result of a condition. The convention exit 1 = false from an earlier discussion is no longer in use.
- Do not store the current owner, focus, portal target, or pairing private keys in the config. These items are runtime state or local state of the computer.
- `lab.http` only shows the extension format. No HTTP driver is implemented. All example helpers are also placeholder paths.

JSON Schema validates only the structure. Semantic validation must find duplicate ids, dangling targets, and handoffs to the same computer. Semantic validation must also check whether the target connects to the same device. Driver availability and permissions are an additional capability check. The offline validator does not run commands.

## Recommended implementation architecture

Use a shared Rust core + winit windowing + egui UI + per-platform native input/clipboard backends. This is a recommendation. The user did not specify it as a technology that cannot change. First, validate the main constraints with prototypes. Then lock the dependency versions.

Do not treat window keyboard events or global hotkey registration as complete input interception. You must implement capture, suppress, forward, and inject separately. macOS Accessibility/Input Monitoring is required. Windows has UIPI and secure desktop restrictions. On Wayland, the compositor/portal capabilities are different. You must not claim that the application can intercept all system shortcuts.

Input callbacks must not run helpers, wait for the network, or block the UI. The hot path uses bounded queues. The down/up events for keys and mouse buttons must be paired. The receiving computer and the source computer both need a cleanup after a disconnect. Network connections must have identity pairing and encryption. You must not accept an injection request only because the sender is on the LAN.

## Runtime flow

1. The user selects the local computer ID. Read and validate the config. Detect the local capabilities.
2. Create/move the portal. Evaluate the local portal-match. If there is no match, tell the user. If the match is ambiguous, ask the user to choose. Do not silently use the first match.
3. Make the candidate targets from the shared id and the local handoff routes. Some entries are offline, do not have permissions, or have no input-target. You must clearly explain these entries. These entries must not show as available.
4. After the selection of a target, the receiving computer resolves the local input-target by the shared id. The receiving computer also confirms the injection capability and an exclusive session lease.
5. Stop the old input stream and release the held state. Run the handoff steps/verify on the initiating computer. After success, the receiving computer shows the confirmation prompt on its input-target display. Then the user presses the confirmation key on the source computer (`docs/adr/0002-user-confirms-handoff.md`). Enable forwarding only after this user confirmation and after you confirm that the display token is valid.
6. Input focus follows the portal region. The Mac stays the capture computer, also when the display already shows the PC. It must not require the window focus events of the PC to know where to send input.
7. Stop suppression on one of these events: the release shortcut, a disconnect, lease expiry, display removal, or permission revocation. Then restore the local cursor and the key state. If the video input is not confirmed, mark it as unknown. Do not claim automatic recovery.

The display owner and the input focus are separate. Confirmed by the user (2026-10-27, ADR 0002): after an exit from the portal, only input goes back to the Mac. The display continues to show the picture of the PC. When the pointer comes back into the portal, forwarding starts again with no new confirmation. Not decided: whether an exit from the portal also switches the video input back automatically (see the open items). Show this behavior clearly in the UI. There should be an operation entry point to switch the video input back to the Mac. This entry point should be usable from the built-in display.

## MVP and later work

First, build a complete vertical slice. In this slice, the Mac is the source computer and Linux is the receiving computer. The architecture keeps space for bidirectional operation and for Windows. You must record the verified environments for Linux X11 and Linux Wayland separately. You must not use success on X11 to claim full Linux support.

Recommended order:

1. Validate parser/schema/semantics and driver I/O. Supply a diagnostics command. Test helpers that return false, errors, timeouts, and invalid JSON.
2. macOS window prototype: borderless and fullscreen. No control buttons show at the top. It is possible to exit/release it safely on a configuration with two displays.
3. Input prototype: capture and suppress ordinary keys, modifier keys, and the mouse. Do Linux injection. Include the release of control and a watchdog from day one.
4. Switch the video input of a real display, and add the input-target on the receiving computer. Confirm whether the Mac keeps the display after the switch. Handle the case where the display is lost.
5. Integrate portal candidates, handoff, focus, and the session state machine. Scope: multiple computers, multiple displays, competing leases.
6. Add opt-in sync of the text clipboard. After that, add the other platforms and the packaging CI.

Not part of the v1 commitment: video streaming, file/image clipboard, atomic switching of device groups, arbitrary shell RPC, all system keys on every OS, login screen/secure desktop control, emulated displays, game relative-mouse mode, cross-platform IME translation.

## Acceptance criteria

| Scenario | Expected |
| --- | --- |
| Desktop / Monitor1 / MyScreen use the same id | The application finds the same physical device correctly. It does not compare the aliases for equality |
| PC1 and PC2 share a second display. The Mac does not connect to it | The list of switch targets for the second device does not include the Mac |
| Click PC1 in the portal on the external display | The corresponding driver runs on the Mac. The physical display shows PC1. Input goes to the selected display on PC1 |
| Click the switch button of PC1 | The click for the switch does not go through to the remote computer as a click |
| Handoff completes; PC1 shows the prompt; the user presses the confirmation key in the focused portal | Forwarding starts. The confirmation key does not go to PC1. The UI shows the video input as confirmed |
| The confirmation key is pressed while no prompt is visible, or while the portal does not have the window focus | It is a normal key. Forwarding does not start |
| No confirmation in the timeout | The wait stops. PC1 removes the prompt. The UI shows the video input as unknown. The video input does not switch back |
| The source portal moves to another display | Do the match again. The previous device binding and incorrect coordinates must not continue to apply |
| condition false / multiple true / timeout | The result is no match / ambiguous / error, respectively. The application does not select a device arbitrarily |
| handoff fails before it is complete, or verify is false | The application does not enable new forwarding. It does not do a blind retry. It does not claim a rollback |
| The mouse moves back to the built-in display of the Mac | Input goes back to the local computer. No modifier keys or mouse buttons stay pressed on the remote computer |
| Resolution, DPI, rotation, negative coordinates, and different aspect ratios | The mapping of the target cursor is correct. The behavior out of bounds is defined |
| Ordinary shortcuts and modifier key combinations | Supported combinations have an effect on the remote computer. They do not have an effect on the local computer at the same time. The release shortcut is preserved |
| System-reserved shortcuts / insufficient permissions | The application clearly reports the platform limits. It does not claim a successful interception |
| Disconnect, process termination, remote hang, queue overflow | The watchdog releases the input state and restores local control |
| Mac fullscreen near the top edge / Dock / switching Spaces | Record the actual behavior. The goal is that no control bar appears in the way, and that the return to the local computer works normally |
| A switch of the video input causes the Mac display to disappear | Stop forwarding and restore the UI. Do not continue to suppress input in the window that moved to the built-in display |
| Two controllers want to control the same device at the same time | The lease prevents forwarding at the same time. The local state of each computer alone must not decide the owner |
| Clipboard enabled/disabled and disconnect | Text sync occurs only in an active paired session. No loops occur. After the clipboard is disabled, no more data is sent |
| Generic USB device | The device can be handed off through the device UI. It does not need to act as a display |

You must measure performance: p50/p95/p99 for capture→receive→inject, event loss, and idle CPU/GPU. Network latency measurements must state the clock synchronization method. This round did not specify hard millisecond thresholds. Do not make up benchmarks.

## Still requires prototyping / confirmation

- The physical display model and the wiring. Whether the video input connections stay connected. Whether the switch control channel is still usable while the display is on an inactive video input.
- The target Linux compositor, its version, and its input capture/injection/clipboard support.
- The final combination for the release shortcut. Which special shortcuts to support or map.
- Whether an exit from the portal should automatically switch the video input back. The current tentative decision: do not switch.
- Pairing UX, lease arbitration and the recovery protocol, and the minimum versions for packages on each OS. You must define these items during implementation. The config schema does not already solve them.

These items do not prevent you from building the core and the prototypes first. But you must record test results and a support matrix before you declare the features complete.
