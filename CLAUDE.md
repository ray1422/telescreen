# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Working agreement

- Talk to the user in Taiwanese Traditional Chinese (台灣繁體中文). Write all repository files (docs, code comments, commit messages, Codex prompts) in English.
- Write English text to ASD-STE100 (Simplified Technical English): one instruction or one statement in each sentence, active voice, imperative for procedures, short sentences, one word for one meaning. Do not use Codex/ChatGPT output as a model for style or grammar.
- The project is at the start and the spec has open questions. If the docs do not specify a behavior, ask the user. Do not make the decision silently. Do not show a tentative default as a confirmed user preference.
- The repository is public (open source from the start). Do not commit personal data: email addresses, IP addresses, host names, home-directory paths, user names, device serial numbers, or UUIDs. Local facts go in `CLAUDE.local.md` (gitignored). Read it if it exists. Before each commit, scan the staged diff for personal data.
- Claude owns the design, tests, dependencies, review, and commits. Codex is only a feature implementer ("typist"). See [Delegating to Codex](#delegating-to-codex).

## Current state

The repository has only the spec. There is no application code. Read these files in this sequence before you design:

1. `HANDOFF.md`: requirements, confirmed decisions, runtime flow, MVP sequence, acceptance table, open questions.
2. `docs/config.md`: the normative config and driver I/O contract (protocol 1). This file and `schema/config.schema.json` are the source of truth.
3. `docs/architecture.md`: the recommended stack and the input/window limits of each platform, with sources.
4. `examples/config.yaml`: the reference config. The helper paths are placeholders. `lab.http` is not an implemented driver.

Spec tools (Python, validation only; they do not execute commands from the config):

```sh
uv run --with pyyaml --with jsonschema python tools/validate_config.py examples/config.yaml
uv run --with pyyaml --with jsonschema python tools/test_config.py
```

To run one test case, go to `tools/` and run `uv run --with pyyaml --with jsonschema python -m unittest test_config -k <name>`. JSON Schema checks only the structure. `validate_config.py` does the semantic checks: duplicate ids, dangling handoff targets, self-handoff, a target that does not connect to the same device id, absolute argv paths. If you change the config contract, change `docs/config.md`, the schema, the validator, and the contract tests together.

## Design records

- When a spec question is open, use the `grilling` skill to ask the user. Use `domain-modeling` to record the result. Both are in `.claude/skills/` (MIT, from mattpocock/skills).
- Terms go in `GLOSSARY.md` at the repository root. Make it when the first term is settled. Hard-to-reverse decisions go in `docs/adr/NNNN-slug.md`.
- `docs/adr/0001`: transport is an abstraction (reliable channel + unreliable pointer channel). v1 is a direct LAN connection with keys pinned at pairing. WebRTC/NAT traversal is not part of v1.
- `docs/adr/0002`: after a handoff, the receiving computer shows a prompt, and the user confirms with a key in the focused portal on the source computer. Forwarding starts only after this confirmation. It is a manual `verify`.
- `docs/hardware.md`: measured facts about the user's display and the Linux receiving computer (niri: no portal injection; uinput is available).

## Key concepts (spread across the docs)

- The config is YAML, nested computer → devices. A device map key is a local alias. It has no OS meaning. The shared `id` links the same physical device across computers. Only the listed connections exist. Handoff has a direction.
- The config has no portals. The app makes portal windows at runtime. Runtime state (owner, focus, target, pairing keys) is never in the config.
- `portal-match`, `input-target`, `handoff.<target>.steps`, and `verify` are abstract operations. Drivers implement them. The core does not hard-code display identification (name/EDID/serial). `command` is the only baseline driver: argv only, no shell, JSON on stdin/stdout. Each non-zero exit is an error. JSON gives the value false.
- The physical display owner and the input focus are two different states. The source machine (Mac) stays the capture host. The input focus follows the portal region.
- Input has four different capabilities: capture, suppress, forward, inject. A hook callback only captures, decides, and pushes to a bounded queue. It does not call helpers, does not use the network, and does not block. Key/button down and up events must stay in pairs. On disconnect, lease expiry, display removal, or queue overflow, both ends must release all held keys and buttons. Always keep a local release shortcut. Never forward it.
- Do not claim support for a platform that you did not test (X11 is not all of Linux; Wayland depends on the compositor and portal). Do not invent benchmark numbers.

## Implementation plan (Claude's decision; revise it with the user)

Use a Rust Cargo workspace under `crates/`. Add each crate when an MVP step needs it:

- `telescreen-core`: config parsing and semantic validation (same results as `tools/validate_config.py` and its tests), shared-id resolution, driver registry, the `command` driver runner, the session/lease state machine. No platform code.
- `telescreen-transport`: paired, encrypted, long-lived connection, heartbeat, session generation.
- `telescreen-platform-macos`, `telescreen-platform-linux` (later `-windows`): capture/suppress/inject, cursor, display topology, clipboard, capability reports.
- `telescreen-app`: winit + egui binary: portal windows, target list, status, release UI.

The first step is `telescreen-core` config parsing/validation and driver I/O (HANDOFF MVP step 1). When the workspace exists, record the build and test commands here. At this time there are no cargo commands.

## Delegating to Codex

- Codex is only a typist. Give it code to implement only through the `codex-delegate` skill (`.claude/skills/codex-delegate/`). The skill has the rules, the procedure, the prompt template, and the `codex-wt` script (prepare, run, gate, integrate, cleanup).
- Do not start `codex:rescue` or `codex:codex-rescue` on your own initiative. They give Codex write access with a rewritten prompt and no gate.
- Each Codex task has its own worktree in `../telescreen-wt/<task>` and its own `target/`, cloned from the main `target/`. The Codex sandbox can write only in the worktree. Thus do not use a shared `CARGO_TARGET_DIR`, and do not use symlinks to paths outside the worktree: the macOS sandbox resolves the real path and blocks the write (tested).
- Verify each Codex result on main, with the main `target/`, after `codex-wt integrate` and before the commit.
