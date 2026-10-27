---
status: accepted
---

# The destination verifies a handoff, with a verifier

Status: accepted (user, 2026-10-27). It replaces ADR 0002 in part, from v0.3.0.

ADR 0002 put the check after a handoff in the route of the computer that runs the steps, and the prompt on the target. A hand back breaks this: the target runs the route back, but only the computer that gets the display (the source of the session) can show a prompt on it. Many tools also cannot read the video input back (BetterDisplay `get` returns no value on the LG display of the user). Thus the check belongs to the computer that the device switches to, the **destination**, and it is the same for a handoff and a hand back.

Decision:

- `verify` is in the device section of the destination. The destination runs it after the steps. A route has no `verify`. No `verify` means no check: the video input shows "unverified".
- A verifier is a program that talks with the core in JSON lines. It can show text on the switched display (`{"show":…}`) and ask for an acknowledgement (`{"want":"ack"}`). The core answers `{"ack":true}`. The verifier answers ok or not ok. It does not know how the user acknowledged. The purpose is to make sure that a user sees the switched display.
- The core decides which user actions acknowledge. If the computer that started the handoff has a portal, they are Return and a click in that portal, sent over the network. Else they are Return and a click on the prompt window of the destination. A hand back always uses the destination, because the session already ended.
- The built-in `prompt` verifier shows the prompt with the hint in its own window, asks for an acknowledgement, and answers ok. It answers not ok after 60 seconds.
- A failed verification makes the device state "unknown" on the destination. "It is back" in the device list sets it to confirmed. The device state does not depend on a session.

Rejected: a check on both computers (`verify` on the route and `arrival` on the device). It gave two places for one question, and the route check cannot see the display on most hardware.

## Consequences

- The v0.1.0 prompt becomes the `prompt` verifier. The example configs add it explicitly to keep the current behavior.
- The `command` driver protocol gets a JSON-lines mode for `verify`.
- A verifier that needs other input (for example a Bluetooth keyboard state) reads it itself on the destination. The protocol has no other user input.
