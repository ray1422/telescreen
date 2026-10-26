# Telescreen

Telescreen coordinates device switching and keyboard/mouse forwarding through a portal window. It is for environments that have more than one computer and physical connections.

Status: v0.1.0 is in development. The code builds and the automated tests pass. The hardware acceptance tests (`docs/spec.md` section 9) did not run yet, so no platform is supported yet (section 8.2).

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
5. On the source, open a portal on the shared display. Select the target. Then press Return in the portal (or click it) when the target shows the prompt.

To go back to the source: move the pointer out of the portal to a different display (pause), or press the release shortcut (end): Control+Option+Command+Escape on macOS, Control+Alt+Shift+Escape on Linux. The safety exit is Escape 10 times in 5 seconds.

macOS asks for Accessibility and Input Monitoring. Linux needs write access to `/dev/uinput` (for example the `input` group) and a compositor with the wlroots virtual pointer and layer-shell protocols.

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
