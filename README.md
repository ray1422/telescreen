# Telescreen

Telescreen coordinates device switching and keyboard/mouse forwarding through a portal window. It is for environments that have more than one computer and physical connections.

Status: in development. There is no release yet. The first release is v0.3.0 (`docs/spec.md` section 2.1).

- The code of the internal milestones v0.1.0 (handoff and keyboard and mouse forwarding, macOS and Linux in both directions) and v0.2.0 (text clipboard) is complete. The automated tests pass on macOS and on Linux (niri).
- Hardware tests on one Mac and one Linux computer with niri: a session reaches forwarding in both directions, and the text clipboard tests pass. The acceptance tests of `docs/spec.md` section 9 did not all run, and one bug is open (`docs/test-plan.md` section 6). Thus no platform is supported yet (section 8.2).
- In progress for v0.3.0: hand back, the verification model, and the operation drivers (`docs/plan.md`).

Documents:

- [Specification and acceptance criteria](docs/spec.md)
- [Glossary](GLOSSARY.md)
- [Decisions](docs/adr/)
- [Spike results](docs/spikes.md)
- [Configuration and driver contract](docs/config.md)
- [Technology choices and platform limitations](docs/architecture.md)
- [YAML example](examples/config.yaml)
- [JSON Schema](schema/config.schema.json)

## Build

Rust (stable) and Cargo.

```sh
cargo build --release -p telescreen-app
cargo test --workspace
```

On Linux, the build needs the Wayland and xkbcommon development files. On macOS, `scripts/make-macos-app.sh` makes an ad-hoc signed `Telescreen.app`.

## Use

1. Write one config for all computers (`docs/config.md`, `contrib/example-lg.yaml`). Check it: `telescreen check-config config.yaml`.
2. On each computer, show its fingerprint: `telescreen fingerprint`.
3. On each computer, trust each other computer: `telescreen trust <computer-id> <fingerprint>`.
4. On each computer, run Telescreen with its own computer ID:
   - Linux: `telescreen run --config config.yaml --computer linux`
   - macOS: `open -a Telescreen.app --args run --config "$PWD/config.yaml" --computer mac`
5. On the source, open a portal on the shared display. Select the target. When the target shows the prompt, press Return in the portal (or click it).

To go back to the source, use one of these:

- Move the pointer out of the portal to a different display. Forwarding pauses; the session continues.
- Press the hand back shortcut: Control+Option+Command+Escape on macOS, Control+Alt+Shift+Escape on Linux. The target switches the display back with its route, and the source shows the prompt. Press Return or click on the source to confirm.
- On the target, select "Hand back" in the control window.

The safety exit (Escape 10 times in 5 seconds) only ends the session. It does not switch the display.

macOS asks for Accessibility and Input Monitoring. Linux needs write access to `/dev/uinput` (for example the `input` group) and a compositor with the layer-shell and `ext-data-control-v1` protocols. Tested on niri and on KDE Plasma.

## Troubleshooting

### The pointer on the target stops and then jumps

Cause: the network delays some pointer datagrams, and then they arrive together. On a Mac with Wi-Fi, AWDL (the interface for AirDrop and Continuity) is a known cause. It makes the Wi-Fi leave its channel at intervals. In a measurement (2026-10-28), more than 5% of the datagrams arrived 60 to 90 ms late. With AWDL off, the largest ping time was 3.9 ms.

1. Measure the delay from the Mac to the other computer. Look for times over 20 ms:

   ```sh
   ping -i 0.1 -c 150 <address>
   ```

2. If the Mac uses Wi-Fi, set AWDL off while you use Telescreen:

   ```sh
   sudo ifconfig awdl0 down
   ```

   AirDrop, Continuity, Universal Control, and Sidecar do not work while AWDL is off. macOS can set AWDL on again, for example when you use AirDrop. A restart sets it on.
3. To set AWDL on again:

   ```sh
   sudo ifconfig awdl0 up
   ```

4. Alternative with AWDL on: set the 5 GHz channel of the Wi-Fi access point to 149. In the same measurement, channel 149 removed the delays (largest ping time 6.5 ms), but channel 48 did not. Do not use a DFS channel (52 to 144): the access point must stop for radar detection. The result can be different on a different network.
5. A network cable prevents the problem.

A Telescreen built with the debug protocol shows the pointer statistics of the target (`{"cmd":"pointer_stats"}`, `docs/debug-protocol.md`). Each session also writes a summary to the log when it ends (NET-9).

## Specification tools

Validate the example. This needs Python 3.10+, PyYAML, and jsonschema. The tool does not run the commands in the config:

```sh
uv run --with pyyaml --with jsonschema python tools/validate_config.py examples/config.yaml
```

Python is only for the tools that validate the specification. The recommendation for the application is Rust.

Contract checks. These checks cover dangling targets, duplicate IDs, unknown fields, incorrect driver parameters, and YAML constructs that are not permitted:

```sh
uv run --with pyyaml --with jsonschema python tools/test_config.py
```

## License

BSD 3-Clause. See `LICENSE`. The skills in `.claude/skills/grilling/` and `.claude/skills/domain-modeling/` are from mattpocock/skills under the MIT License. Each of them has its own `LICENSE` file.
