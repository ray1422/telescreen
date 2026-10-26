# Task: <one-line goal>

You are an implementer. Implement only this task. Do not change the design.

## Read

- <spec path and section, e.g. docs/config.md "Driver I/O contract">
- <test file that defines the acceptance, e.g. crates/telescreen-core/tests/config.rs>

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
- Do not add or change tests, `#[cfg(test)]` modules, `#[test]` functions, or fixtures.
- Do not add `unsafe`, FFI, `#[allow(...)]`, or `#[expect(...)]`.
- Do not use git. Do not install tools. Do not use the network.
- Do not work around a problem. If a dependency is missing, a test looks wrong, or the spec is not clear, stop. Then report the problem.

## Done when

- `cargo test --offline` passes with no change to the tests.
- `cargo clippy --offline --all-targets` shows no warnings.

## Report

Write a short report: the files you changed, the commands you ran and their results, and each problem that you did not solve.
