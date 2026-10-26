---
status: accepted
---

# The macOS portal is a native fullscreen Space, locked while forwarding

The user switches between macOS Spaces with trackpad gestures, and wants the portal to be one of these Spaces. Thus the portal uses native fullscreen (a Space in Mission Control), not simple fullscreen. The portal hides the menu bar, the title bar, and the Dock completely, also when the pointer touches the top edge. The presentation options `hideMenuBar | hideDock` (winit `set_borderless_game`) are not sufficient: the menu bar and the title bar still show at the top edge (spike 1). After the window enters fullscreen and has the focus, set its level to 26 (above `NSMainMenuWindowLevel`, 24). Then they do not show (spike 1, `docs/spikes.md`).

While input is forwarded, the display shows the target computer, so the user cannot see the Spaces of the Mac. Thus, while forwarding, the source computer suppresses all gestures and shortcuts that switch Spaces or applications, or that open Mission Control. It does not forward them. Before the confirmation and after the release, these gestures go to macOS as usual. If a gesture cannot be suppressed, the source computer activates the portal again immediately after a Space change.

Pausing forwarding and ending the session are different (user, 2026-10-27):

- **Pause (the session stays).** Move the pointer out of a portal edge that touches a different local display. Input goes back to the source computer, and the user can use the other displays. The target display does not change. When the pointer comes back into the portal, forwarding starts again with no new confirmation (ADR 0002). On macOS, the source computer tracks a virtual pointer while the cursor is frozen. When the virtual pointer crosses such an edge, it releases the cursor and puts it at the matching point of the adjacent display. Portal edges with no adjacent display stop the pointer. The event tap is global, so while the pointer is in the portal, keys go to the target also when a different local application has the focus.
- **End the session.** One of these:
  - Press the release shortcut (macOS: Control+Option+Command+Escape; Linux: Control+Alt+Shift+Escape, user, 2026-10-27; fixed in v0.1.0).
  - The source computer detects that the target computer is disconnected (heartbeat timeout or a closed connection).
  - On the target computer, the user selects "disconnect". The target computer ends the session.
  - The target computer requests control back: it ends the session, and then it can be the source computer in the reverse direction (ADR 0006).
  - Safety exit: press Escape 10 times in 5 seconds. The event tap callback counts the presses itself, with no dependency on the main thread, the UI, or the network. The 10th press stops suppression, restores the cursor, and stops the session. Escape is still forwarded to the target computer.

## Considered Options

- **Simple fullscreen (no Space).** It hides the system UI more reliably. The user rejected it, because the portal must be a Space for gestures.

## Consequences

- An input watchdog is necessary: if the main loop does not respond for a few seconds, the event tap lets all events through.
- Spike 1 must show that the menu bar does not show in the fullscreen Space on the current macOS. Spike 2 must show which gesture events an event tap can suppress, and that the cursor comes back after a crash or a kill.
- Later work (not v0.1.0): send Mac three-finger swipes to the receiving computer as niri workspace actions.
