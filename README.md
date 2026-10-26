# Telescreen

Telescreen coordinates device switching and keyboard/mouse forwarding through a portal window. It is for environments that have more than one computer and physical connections.

Status: the specification and the technology spikes are done. The application is not implemented yet. The first release will be v0.1.0.

- [Specification and acceptance criteria](docs/spec.md)
- [Glossary](GLOSSARY.md)
- [Decisions](docs/adr/)
- [Spike results](docs/spikes.md)
- [Configuration and driver contract](docs/config.md)
- [Technology choices and platform limitations](docs/architecture.md)
- [YAML example](examples/config.yaml)
- [JSON Schema](schema/config.schema.json)

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
