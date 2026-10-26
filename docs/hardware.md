# Hardware notes

This file records measured facts about the user's hardware. Each fact has a date and the method. Do not add facts that were not measured.

## External display "Desktop" (Mac side)

Measured on 2026-10-27 with BetterDisplay `get -namelike=Desktop -identifiers`:

| Field | Value |
| --- | --- |
| Name in BetterDisplay | `Desktop` |
| Vendor | 7789 (0x1E6D, LG) |
| Model | 30457 |
| Year of manufacture | 2018 |
| Connection | `dispext0` (external display controller of the Mac) |

### Input switch

The user gave this command to switch the input of the display:

```sh
/Applications/BetterDisplay.app/Contents/MacOS/BetterDisplay set -namelike=Desktop -ddcAlt=0x91 -vcp=inputSelectAlt
```

- `ddcAlt` is the alternate DDC addressing that BetterDisplay uses for the video input of some LG displays.
- User statement (2026-10-27): `0x91` selects the Mac and `0x90` selects the Linux computer. The user reported that the switch works.

## Linux receiving computer (niri)

Measured on 2026-10-27 over SSH, read-only:

- niri 26.04 (Wayland). The same LG display is on `HDMI-A-2`: 2560x1080, scale 1.25, logical size 2048x864.
- Portal backend: xdg-desktop-portal-gnome. `org.freedesktop.portal.RemoteDesktop` reports `AvailableDeviceTypes = 0`. `org.freedesktop.portal.InputCapture` reports `SupportedCapabilities = 0`. niri does not provide `org.gnome.Mutter.RemoteDesktop`. Thus input injection through the portal or libei is not available on this setup.
- `/dev/uinput` is `root:input` with mode `0660`. A user in the `input` group can make a uinput device. Thus uinput injection is available.

### Input read (for `verify`)

Measured on 2026-10-27. Both reads fail. BetterDisplay writes `Failed.` and exits with 1:

```sh
BetterDisplay get -namelike=Desktop -ddcAlt -vcp=inputSelectAlt
BetterDisplay get -namelike=Desktop -ddc -vcp=inputSelect
```

Decision (user, 2026-10-27): the handoff of this display has no `verify`. A handoff is complete when the switch command exits with 0. A DDC write does not report if the display really changed the video input. Thus the app does not know the video input of the display after a handoff. HANDOFF runtime step 7 says: if the video input is not confirmed, mark it as unknown. `docs/config.md` runtime step 5 says: the UI must show "confirmed" and "unverified" as different states.

### BetterDisplay CLI exit codes

Measured on 2026-10-27:

- A successful `get` writes the value and exits with 0.
- A failed `get` (unknown display, or a DDC read that fails) writes `Failed.` and exits with 1.

BetterDisplay output is not the JSON of driver protocol 1. A `command` driver needs a helper that runs BetterDisplay and writes the protocol 1 result.

## Open questions

- After the display switches to a different video input, does the Mac still list the display? (HANDOFF acceptance: "A switch of the video input causes the Mac display to disappear".)
- When the display shows a different video input, can the Mac still send DDC to switch the input back?
