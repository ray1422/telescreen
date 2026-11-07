---
status: accepted
---

# The source can request a hand back from the target

Status: accepted (user, 2026-10-27).

`docs/config.md` says that each operation runs on the computer that it belongs to, and that no computer gives the execution to a different computer. A hand back on the source breaks this rule: when the user selects "Hand back" for a portal on the source, the display shows the target, so only the target's route can switch it back. Without a request, the user must touch the button of the display.

Decision: the source of an active, authenticated session can send a hand back request. The target then runs its own `handoff.<source>` route, as for its own "Hand back" button. The request names no command and no route: it can only cause the route back to the peer that sent it. The route is in the config of the target, so the target still runs only its own operations. If the target has no such route, the session ends, and the target shows a notice that the user must switch the display by hand.

## Consequences

- The wire protocol gets a hand back request.
- The release shortcut becomes the hand back shortcut. The safety exit only ends the session.
- Amendment (user, 2026-11-07): the hand back shortcut is removed (IN-7). The only trigger on the source is "Hand back" in the session row of a portal (HB-3). A portal that leaves fullscreen pauses forwarding and does not request a hand back (UI-3).
- A peer that is not in an active session with the target cannot cause an operation on it.
