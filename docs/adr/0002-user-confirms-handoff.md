---
status: accepted
---

# The user confirms a handoff with a key on the source computer

A handoff runs an external command that switches the video input. Often the computer cannot read the video input back (see `docs/hardware.md`). Thus forwarding does not start when the handoff command completes. The receiving computer shows a prompt ("Press <key> to take control") on its input-target display. The user sees the prompt only if the display shows the receiving computer. The portal window on the source computer stays open and keeps the window focus. Thus the user presses the confirmation key on the source keyboard, and the portal receives it as a normal window key event. Only then does the source computer start to capture and forward input.

Decisions (user, 2026-10-27):

- The prompt also shows the optional `hint` of the receiving computer from the config, as plain text (for example its copy/paste shortcuts). Telescreen does not remap shortcuts such as Command+C (user, 2026-10-27).
- The confirmation is one fixed key. In v0.1.0 the key is fixed (Return). A later version makes it configurable (user, 2026-10-27). A click in the portal also confirms. The source computer suppresses the confirmation key and the confirmation click. It does not forward them.
- The source computer accepts the confirmation only while the receiving computer reports that the prompt is visible and the portal has the window focus. At other times the key is a normal key.
- If there is no confirmation in 60 seconds (fixed in v0.1.0; configurable in a later version), the source computer cancels the wait. The receiving computer removes the prompt. The UI shows the video input as unknown. The video input does not automatically switch back.
- After the confirmation, input goes back to the source computer when the pointer leaves the portal. The display continues to show the receiving computer. When the pointer comes back into the portal, forwarding starts again with no new confirmation, while the session is valid and no new handoff ran.

## Consequences

- The confirmation is a manual `verify`: after it, the UI can show the video input as confirmed.
- The click on the switch button cannot go to the receiving computer, because forwarding starts only after the confirmation.
- Before the confirmation, the source computer does not need input capture permissions.
- The receiving computer needs a UI that can show the prompt on the input-target display, not only an input injector.
- The design needs the source computer to keep the display after the video input changes. The user reported that the switch works on the current hardware. The display list of the Mac after a switch is not measured.
