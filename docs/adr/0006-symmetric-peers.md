---
status: accepted
---

# All computers are symmetric peers

Each computer runs the same Telescreen and has both roles: it can be the source of a session (it opens a portal and forwards its input) and the target of a session (it injects input and shows the confirmation prompt). Each peer listens on the `endpoint` of its own computer in the config. The source dials the target. There is no server and no client (user, 2026-10-27). Use the terms in `GLOSSARY.md`: peer, source, target, dialer, listener.

v0.1.0 has both directions. Make Mac to Linux work and verify it first. Then make Linux to Mac work. v0.1.0 is complete only when both directions work.

Linux as the source (verified protocols on niri, `docs/spikes.md`): the portal is a fullscreen window. After the confirmation, it locks the pointer (`zwp_pointer_constraints_v1`), reads deltas (`zwp_relative_pointer_manager_v1`), and inhibits compositor shortcuts (`zwp_keyboard_shortcuts_inhibit_manager_v1`). It needs no root and no `/dev/input` access. A crash releases all of them, because the compositor removes them with the window. Wayland does not let a client move the pointer. Thus, to pause at a portal edge, the portal unlocks the pointer with a position hint at that edge, and the compositor moves the pointer from there.

macOS as the target: inject with `CGEventPost` (Accessibility).

## Considered Options

- **Server and client.** Rejected by the user: every computer must be able to control every other computer.
- **offerer and answerer.** Rejected: these are WebRTC SDP terms, and ADR 0001 keeps WebRTC as a possible later backend.

## Consequences

- Injection is a replaceable backend. v0.1.0: wlroots virtual pointer (niri, sway, Hyprland) plus a uinput keyboard on Linux, `CGEventPost` on macOS. Later: libei through the RemoteDesktop portal (KDE, GNOME). Do not claim support for a compositor that you did not test.
- The confirmation prompt on Linux uses layer-shell. Compositors with no layer-shell (GNOME) need a normal fullscreen window instead.
- Both peers authenticate each other (ADR 0004).
