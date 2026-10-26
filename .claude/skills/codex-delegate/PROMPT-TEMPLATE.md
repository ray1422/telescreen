# Task: <one-line goal>

You are an implementer. Implement only this task. Do not change the design.

## Read

- <spec path and section, e.g. docs/config.md "Driver I/O contract">
- <test file that defines the acceptance, e.g. crates/telescreen-core/tests/config.rs>

## Code quality

- Keep the responsibility boundaries of the design: <name the modules and what each one owns>.
- Use the terms in `GLOSSARY.md` for names (peer, source, target, dialer, listener).
- Use explicit error types. Do not `unwrap` or `expect` on input that can fail.
- Do not add a dependency, a feature flag, or a new module that this task does not name.

## Do

<Numbered steps. One instruction in each step. Name the public types and functions that the tests use.>

## Files you can change

Change only these files. You can make a file in this list if it does not exist.

- <path>
- <path>

## Commands you can run

- `cargo build --offline`
- `cargo check --offline --all-targets`
- `cargo test --offline`
- `cargo clippy --offline --all-targets`
- `cargo fmt`

## Prohibited

- Do not change any other file.
- Do not change a `Cargo.toml`, `Cargo.lock`, any `*.toml`, `build.rs`, `.cargo/`, or `rust-toolchain*`.
- Do not change, delete, weaken, or disable an existing test (no `#[ignore]`, no removed `assert!`). The existing tests are the acceptance spec.
- <ONE OF:> Do not add tests. | You can add unit tests for your own code, in the files above. They must test behavior through the public interface of the module.
- Do not add `unsafe`, FFI, `#[allow(...)]`, or `#[expect(...)]`.
- Do not use git. Do not install tools. Do not use the network.
- Do not work around a problem or a rule. If a dependency is missing, a test looks wrong, the spec is not clear, or two rules conflict, stop. Then report the problem, with the file, the line, and what you need. Do not change the test or the rule to make the code pass.

## Done when

- `cargo test --offline` passes with no change to the tests.
- `cargo clippy --offline --all-targets` shows no warnings.

## Report

Write a short report: the files you changed, the commands you ran and their results, and each problem that you did not solve.
