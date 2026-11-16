# 0015: A login stage sends keys to the login screen of a Linux target

Status: accepted (user, 2026-11-16 and 2026-11-17).

## Context

Telescreen on a Linux peer starts only after the user logs in to the desktop. Thus the user needs the hardware keyboard of that peer to type the password on the login screen. The user wants to log in from the source with no hardware keyboard on the target.

Constraints from the user:

- No automatic login. The wallet of the desktop opens with the password that goes through PAM at the login. Thus the password must go into the login screen itself.
- No service that runs all the time. The user starts the function when it is necessary.
- The picture of the login screen is not part of the first version. The shared display shows the login screen after the switch.
- The first version is a macOS source and a Linux target (user, 2026-11-17). A macOS login screen does not accept input from a user process.

Measured on the test computer (2026-11-17):

- A process that the user starts through SSH can make a uinput keyboard, and the login screen gets its keys.
- logind sends `SessionNew` for the desktop login. The properties of the session (`Class=user` with a seat) separate it from an SSH login.
- Desktop programs start less than 1 second after that event.

## Decision

- A **stage** is a state of a computer before its desktop session, in which Telescreen can control it in a limited way. The kinds are built in. The only kind is `login`: the login screen of a display manager.
- The config has `stages.login.from` in the section of the computer that is controlled. `from` lists the source computers that can control the stage. A trusted fingerprint does not authorize (NET-4), and a stage has no handoff route, so the stage has its own list.
- The user starts the stage on the target by hand: `telescreen run --stage login`, for example through SSH. The process runs with the account of the user. It uses the usual identity, trust list, config, and endpoint port. It opens no window. It makes the computer discoverable, and it prints a notice about this.
- A stage session sends keys only. The target enforces this: it does not inject a pointer position, a button, or a scroll, and it does not sync the clipboard. The target needs no `input-target`, no display, and no keyboard focus holder. The session has no `verify`, so the device state is unverified.
- The source action is "Switch & log in". A hint window shows the command and waits. When the target connects in the login stage, the window shows a button. The button runs the handoff steps of the source for the device, opens the portal on that display, and starts the stage session.
- The target finds the desktop login from logind. Then it ends the stage session with a reason for this, releases all keys, and stops, so that the Telescreen of the desktop session can use the port.
- After the login, the source keeps the portal open and keeps the keys on the source. It waits, with no time limit, until the Telescreen of the desktop session connects. Then it continues as an attach ("Control only"). The user can end the wait with "End session" or "Switch back".
- The Telescreen of the desktop session has priority. The stage process does not take the single-instance name. It records its process ID. If the Telescreen of the desktop session cannot bind the port, it stops the recorded stage process and binds again. The stage process does not start if the user already has a desktop session or if the port is in use.
- The wire protocol becomes version 7. The handshake tells the stage of the peer, and a session end has a reason for a completed login. All peers must update.

## Alternatives that we did not use

- A system service that starts at boot: it runs all the time, and the identity and the trust list are in the home directory of the user.
- Automatic login, then a locked screen: the wallet does not open, and the session starts before the password.
- A login through SSH with no login screen: the password does not go through PAM, so the wallet does not open.
- A different display manager with a login protocol (greetd): the user must replace the display manager.
- The accessibility interface of the login screen: a screen reader does not start on this login screen, so the interface is not reliable.
- Key injection through the X server of the login screen: it does not work on a Wayland login screen.
- A picture of the login screen (X11 capture, KWin capture, or DRM/KMS capture): each path depends on the login screen or on the GPU driver. It can come later as a separate function.

## Consequences

- The password goes over the network as key events, inside the mutually authenticated connection, as all other keys do.
- While the stage process runs, each computer in `from` with a trusted key can send keys to the login screen. The user started the process, and it stops after the login, when the terminal closes, or when the user stops it.
- The source cannot see the login screen through Telescreen. The user sees it on the shared display after the switch.
- The keyboard layout of the login screen applies to the keys, not the layout of the desktop session.
- If Telescreen does not start in the desktop session, the source waits until the user ends the session. A setting for the start at login is not implemented yet.
- A later stage kind (for example a boot loader) is a new built-in key. Software cannot get input before the operating system starts, so such a kind needs a different driver.
