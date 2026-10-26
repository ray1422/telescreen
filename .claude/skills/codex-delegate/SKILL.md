---
name: codex-delegate
description: Procedure and scripts to give one bounded implementation task to Codex in an isolated git worktree, then gate, verify, and merge the result. Use each time Claude delegates code to Codex in this repository. Do not use codex:rescue or codex:codex-rescue for this.
---

# Delegate an implementation task to Codex

Codex is a typist. It implements code that Claude specified and tested. Codex works around restrictions aggressively. Thus the sandbox, the worktree, and the gate enforce the restrictions. The prompt alone does not.

Script: `.claude/skills/codex-delegate/codex-wt`. Run it from the main checkout. Run `codex-wt` with no arguments to see the usage.

## Rules for Codex

Codex can only:

- Edit, make, or delete the files in the allowlist of the task.
- Run `cargo build`, `cargo check`, `cargo test`, `cargo clippy`, and `cargo fmt` with `--offline`.

Codex must never:

- Change a `Cargo.toml`, `Cargo.lock`, any `*.toml`, `build.rs`, `.cargo/`, or `rust-toolchain*`.
- Add or change tests (`tests/`, `benches/`, fixtures, `#[cfg(test)]`, `#[test]`).
- Change `docs/`, `schema/`, `examples/`, `tools/`, `.claude/`, `.agents/`, `.codex/`, `.github/`, `.gitignore`, scripts, or the top-level `*.md` files.
- Add `unsafe`, FFI, or `#[allow]`/`#[expect]`, unless the task permits it.
- Use git (commit, branch, stash, reset, checkout), install tools, or use the network.
- Work around a blocker. If Codex is blocked, it must stop and report.

The gate rejects all of these items. Some items are on a fixed deny list, so the gate rejects them also when the allowlist names them.

## Procedure

1. **Prepare on main.** Write the crate skeleton, the dependencies in `Cargo.toml`, and the tests. The tests are the acceptance spec. Run `cargo fetch` and `cargo build --tests`, because the Codex sandbox has no network and the cloned `target/` makes the first build fast. Commit this work with the user's approval. The main checkout must be clean.
2. **Make the worktree.** Run `codex-wt prepare <task>`. The task name is `[a-z0-9-]+`. The script:
   - makes `<main>-wt/<task>` on branch `codex/<task>`;
   - clones `<main>/target` into `<main>-wt/<task>/target` with `cp -cRp` (APFS copy-on-write, so it uses almost no disk);
   - records the base commit in `<main>-wt/.meta/<task>.base`, outside the Codex writable root.
3. **Write the allowlist.** Write one repository-relative path on each line, in a file outside the worktree (for example the scratchpad). Lines that start with `#` are comments. Do not use globs.
4. **Write the prompt.** Copy `PROMPT-TEMPLATE.md` to a file outside the worktree. Fill in all the fields. Write it in ASD-STE100 English.
5. **Run Codex.** Run `codex-wt run <task> <prompt-file>`. Add `--background` for a long task. The script sets `CARGO_TARGET_DIR=<worktree>/target` and runs the companion with `--write --cwd <worktree>` (sandbox `workspace-write`, approval `never`).
6. **Gate.** Run `codex-wt gate <task> <allowlist-file>`. Add `--allow-unsafe` or `--allow-lint-allow` only if the task permits it. If the gate fails, do not accept the result "because it works". Revert the rejected paths, or run `codex-wt cleanup <task>` and redo the task with a better prompt.
7. **Review.** Read the full diff: `git -C <main>-wt/<task> diff`. Check the code against the spec and the tests. Do not trust the report from Codex.
8. **Integrate.** Get the user's approval. Then run `codex-wt integrate <task> "<commit message>"`. The script commits on the task branch and merges into main with `--no-commit`.
9. **Verify on main.** Run the full build and the full tests in the main checkout, with the main `target/`. Do not use the artifacts from the worktree. If the result is good, run `git commit`. If it is not good, run `git merge --abort`.
10. **Clean up.** Run `codex-wt cleanup <task>`.

## Verification

Run `./selftest.sh` after a change to `codex-wt`, and on each new machine. It uses a temporary repository, does not call Codex, and does not use the network. It must show `selftest: all checks passed`.

## Known limits

- `cp -c` needs APFS. On a different file system, `prepare` fails at the copy. Then remove the clone step or use a plain copy.
- The gate checks paths and added lines. It does not find all possible workarounds. Step 7 is necessary.
- The `codex` CLI is in `/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin`. It is not on `PATH`. `codex-wt run` adds this directory to `PATH`. Set `CODEX_BIN_DIR` to use a different directory. To check the setup, run `PATH=<that dir>:$PATH node <companion> setup`.
