---
status: accepted
supersedes: 0005
---

# On macOS, the network runs in the app process

Decision (user, 2026-10-27): remove the network helper process of ADR 0005. The network runs in `Telescreen.app`, as on Linux. Do not use `responsibility_spawnattrs_setdisclaim` as a workaround. If the installed app cannot connect because of Local Network privacy, find the cause and solve it directly.

## Why ADR 0005 was wrong

- The source of ADR 0005 is a different project (not-enough-tokens). That project disclaims `/usr/bin/ssh`, a system command-line tool that is not in an app bundle. Command-line tools are not subject to Local Network privacy, so the fix works there. Telescreen would disclaim its own ad-hoc signed executable in the app bundle. No evidence shows that macOS treats that executable as a command-line tool. The implementation started the main executable of the app, which macOS can identify as the app.
- The problem did not occur in Telescreen. In spike 4 (`docs/spikes.md`), a locally built, ad-hoc signed app bundle got the Local Network prompt, and the connection completed after the user allowed it. The failure was seen only in the other project: an Electron app installed from a dmg.
- `POSIX_SPAWN_SETEXEC` in ADR 0005 came from the trampoline of the other project. Telescreen did not need it.
- The helper added a process, a Unix domain socket, a JSON protocol, and one more failure path for AC-18. It gave no measured benefit.

## Consequences

- One process on macOS. The controller uses the transport directly, as on Linux.
- The app keeps `NSLocalNetworkUsageDescription` in `Info.plist`.
- AC-25 is open. Test the real distribution path before a release: a dmg, downloaded with `com.apple.quarantine`, dragged to Applications, then started (`docs/spikes.md`, release checks). A local build does not show this path.
