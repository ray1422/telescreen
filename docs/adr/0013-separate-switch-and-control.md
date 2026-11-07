# 0013: Switch and control are separate actions, and a target can ask for control

Status: accepted (user, 2026-11-06).

## Context

Before this decision, one action did three things: it switched the device (the handoff steps), it made a session, and it forwarded the input. Only the source could start it, from its portal. This caused three problems (user, 2026-11-06):

1. A portal stays open, and the user cannot find it.
2. When the user switches the display by hand to a different computer, no computer can control that computer. The source must run the steps again, and the user cannot see the portal of the source. This LG display cannot read its video input over DDC, so Telescreen cannot see the manual switch.
3. A switch needs the target to run Telescreen, because the target must grant a lease first (FLOW-5).

## Decision

- A candidate has three actions: "Switch & control" (the handoff and a session, as before), "Switch display only" (the handoff with no session), and "Control only" (a session with no handoff; the glossary calls it an attach).
- "Switch display only" works also when the destination does not run Telescreen.
- An attach runs the `verify` of the target as for a handoff (user, 2026-11-06: it keeps the design simple).
- A peer can ask a different peer to control it on a device: a control request. The peer that receives it opens a portal on the matching display and attaches to the peer that asked. It does not ask the user first, because its own display can show the other computer at that time (user, 2026-11-06). The acknowledgement on the keyboard of the receiving peer is the consent: it proves that a user at that keyboard sees the display of the peer that asked.
- The portal exists only while a session exists.

## Consequences

- The wire protocol gets a control request. It names a device id and nothing else. It cannot cause a command on the receiving peer.
- A paired peer can make a different peer open a fullscreen portal that holds the keyboard focus for up to 60 seconds (FLOW-21, VER-5). The safety exit ends it. This is accepted, because only paired peers can send the request.
- The candidate list moves from the portal to the menu and the main window.
