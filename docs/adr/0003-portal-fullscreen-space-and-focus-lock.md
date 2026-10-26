---
status: accepted
---

# The macOS portal is a native fullscreen Space, locked while forwarding

The user switches between macOS Spaces with trackpad gestures, and wants the portal to be one of these Spaces. Thus the portal uses native fullscreen (a Space in Mission Control), not simple fullscreen. The portal hides the menu bar, the title bar, and the Dock completely, also when the pointer touches the top edge. The presentation options `hideMenuBar | hideDock` (winit `set_borderless_game`) are not sufficient: the menu bar and the title bar still show at the top edge (spike 1). After the window enters fullscreen and has the focus, set its level to 26 (above `NSMainMenuWindowLevel`, 24). Then they do not show (spike 1, `docs/spikes.md`).

While input is forwarded, the display shows the target computer, so the user cannot see the Spaces of the Mac. Thus, while forwarding, the source computer suppresses all gestures and shortcuts that switch Spaces or applications, or that open Mission Control. It does not forward them. From the handoff until the confirmation or the confirmation timeout (FLOW-11), the portal stays fullscreen and fixed in place, and it keeps the window focus: it cannot be switched away (user, 2026-10-27, wireframe step 2). The source computer suppresses the same gestures and shortcuts in this time, and it activates the portal again if it loses the focus. Before the handoff and after the session ends, these gestures go to macOS as usual. If a gesture cannot be suppressed, the source computer activates the portal again immediately after a Space change.

Switching the controlled computer and ending the session are different (user, 2026-10-27):

- **Switch of the controlled computer (the session stays connected).** Move the pointer out of a portal edge that touches a different local display. Input goes back to the source computer at once, and the user can use the other displays. This is a seamless switch, not a pause (user, 2026-10-27). The target display does not change. When the pointer comes back into the portal, forwarding starts again with no new confirmation (ADR 0002). The cursor is always free (user, 2026-10-27): the source does not freeze, lock, or hide it, and pointer motion is never suppressed. The event tap forwards input only while the cursor is in the portal area. Portal edges with no adjacent display stop the cursor, because the OS stops it at the edge of the desktop. The event tap is global, so while the pointer is in the portal, keys go to the target also when a different local application has the focus.
  - Correction (2026-10-27): an earlier version of this ADR said that the cursor is frozen and that the source tracks a virtual pointer from relative motion. That was a proposal of Claude, not a decision of the user. The user's requirement is that the input focus follows the portal region.
  - Each key and button up goes where its down went (IN-3). For example, Shift pressed on a different display and released in the portal goes to the source, not to the target.
  - The cursor shows normally (user, 2026-10-27). The source never hides it. The portal display shows the target, so the source cursor is not in that video signal.
- **End the session.** One of these:
  - Press the release shortcut (macOS: Control+Option+Command+Escape; Linux: Control+Alt+Shift+Escape, user, 2026-10-27; fixed in v0.1.0).
  - The source computer detects that the target computer is disconnected (heartbeat timeout or a closed connection).
  - On the target computer, the user selects "disconnect". The target computer ends the session.
  - The target computer requests control back: it ends the session, and then it can be the source computer in the reverse direction (ADR 0006).
  - Safety exit: press Escape 10 times in 5 seconds. The event tap callback counts the presses itself, with no dependency on the main thread, the UI, or the network. The 10th press stops suppression and stops the session. Escape is still forwarded to the target computer.

## Considered Options

- **Simple fullscreen (no Space).** It hides the system UI more reliably. The user rejected it, because the portal must be a Space for gestures.

## Consequences

- An input watchdog is necessary: if the main loop does not respond for a few seconds, the event tap lets all events through.
- Spike 1 must show that the menu bar does not show in the fullscreen Space on the current macOS. Spike 2 must show which gesture events an event tap can suppress, and that the cursor comes back after a crash or a kill.
- Later work (not v0.1.0): send Mac three-finger swipes to the receiving computer as niri workspace actions.
