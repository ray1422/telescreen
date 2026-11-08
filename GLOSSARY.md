# Telescreen

Telescreen lets computers that share physical devices (displays and other switchable devices) take turns to use one keyboard and mouse. A portal window on one computer switches a shared device and forwards input to a different computer.

## Computers and roles

**Peer**:
A computer that runs Telescreen. All peers are equal, and each peer can be a source or a target.
_Avoid_: server, client, host, node, machine

**Source**:
The peer in a session whose keyboard and mouse control the other peer. It owns the portal.
_Avoid_: controller, master, offerer, initiator, sender

**Target**:
The peer in a session that receives and injects the input.
_Avoid_: receiver, slave, answerer, remote, controlled computer

**Dialer**:
The peer that opens a connection. A transport term only. Usually the source is the dialer, but the two are different concepts.
_Avoid_: client, caller

**Listener**:
The peer that accepts a connection. A transport term only.
_Avoid_: server

## Devices and switching

**Device**:
A physical resource that more than one computer connects to, for example a display or a USB switch. The config identifies it by its shared `id`.
_Avoid_: monitor, screen (for the general concept)

**Alias**:
The local name of a device in the config of one computer. It has no OS meaning.
_Avoid_: device name

**Video input**:
The input that a display shows (for example HDMI 1). A handoff changes it.
_Avoid_: source (this word is the session role), signal source, channel

**Handoff**:
The operation that switches a device to a different computer, by the steps in the config.
_Avoid_: switch (as a noun), takeover

**Destination**:
The computer that a handoff switches a device to. It verifies the handoff.
_Avoid_: receiver, new owner

**Hand back**:
A handoff of the device of a session back to the source: the session ends, the target runs its own route to the source, and the source verifies.
_Avoid_: switch back, revert, rollback

**Verifier**:
The program that the destination runs after a handoff to check that the device arrived, for example that a user sees the switched display.
_Avoid_: checker, arrival

**Acknowledgement**:
The user action (Return or a click) that answers a verifier that asks for one.
_Avoid_: confirmation, approval

**Prompt**:
The built-in verifier: the message on the switched display, with the hint, that asks for an acknowledgement.
_Avoid_: dialog, popup

**Attach**:
A session for a device that already shows the target, with no handoff steps. The target still verifies.
_Avoid_: connect, join, takeover

**Control request**:
A message from a peer that asks a different peer to attach to it as the source, for one device.
_Avoid_: invite, remote control request

**Device state**:
What the destination knows about a device after a handoff: confirmed, unverified, or unknown.
_Avoid_: video input state

**Hint**:
The plain text of a computer in the config that its prompt shows, for example its copy/paste shortcuts.
_Avoid_: tip, note

## Sessions and input

**Portal**:
The fullscreen window on the source that stands for the target. While the pointer is in the portal, the source forwards its input.
_Avoid_: viewer, remote window

**Menu**:
The small panel that lists the sessions, the displays, and the actions. The status item, the Control Center button, and the Plasma applet open it.
_Avoid_: popover, tray menu (for the general concept)

**Main window**:
The window that opens when the user starts Telescreen. It shows the content of the menu, the computers, and the notices. It replaces the control window.
_Avoid_: hub (in code and docs), dashboard

**Session**:
The authenticated relation between one source and one target for one device, from the lease grant to its end.
_Avoid_: connection (that is the transport)

**Lease**:
The exclusive right of one session to control a target device. It prevents two sources at the same time.
_Avoid_: lock, ownership

**Display owner**:
The computer that a display shows at a given time. It is separate from the input focus.
_Avoid_: active computer

**Input focus**:
The peer that receives an input event. Pointer input follows the cursor: it goes to the target while the cursor is in the portal. Keys follow the keyboard focus.
_Avoid_: active computer, owner

**Keyboard focus**:
The window that receives the keys. It changes by a click, as on one computer. While the portal has it, keys go to the target.
_Avoid_: active window

**Focus holder**:
An invisible surface on the target that takes the keyboard focus from the applications of the target while the portal does not have the keyboard focus. Thus only one input field shows the focus.
_Avoid_: dummy window, fake focus

**Forwarding**:
The state in which the source captures, suppresses, and sends its input to the target.
_Avoid_: streaming, sharing

**Hand back shortcut**:
Removed (spec IN-7, user, 2026-11-07). Telescreen has no key combination that starts a hand back. On the source, "Hand back" in the session row of a portal starts it (HB-3).
_Avoid_: release shortcut, release chord, hotkey

**Special key**:
A brightness, volume, media, or keyboard light key that macOS reports as a system-defined event, not as a key event (spec IN-20). The `special-keys` config of the source selects where each group goes.
_Avoid_: fn key, media key (for the whole set)

**Safety exit**:
Escape pressed 10 times in 5 seconds. It ends suppression also when the rest of the program does not respond.
_Avoid_: panic key, kill switch
