#!/usr/bin/env bash
# selftest.sh — prove that codex-wt prepare/gate/integrate/cleanup behave as SKILL.md says.
# Uses a throwaway repository in a temp dir. Does not call Codex and does not use the network.
# Usage: ./selftest.sh   (exit 0 = all checks passed)

set -uo pipefail
tool=$(cd "$(dirname "$0")" && pwd -P)/codex-wt
root=$(mktemp -d "${TMPDIR:-/tmp}/codex-wt-selftest.XXXXXX")
trap 'rm -rf "$root"' EXIT
failures=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; failures=$((failures + 1)); }
expect() { # expect <0|1> <label> <command...>
    want=$1 label=$2; shift 2
    out=$("$@" 2>&1); got=$?
    [ $got = 0 ] && got=0 || got=1
    if [ "$got" = "$want" ]; then pass "$label"; else fail "$label (exit $got)"; printf '%s\n' "$out" | sed 's/^/        /'; fi
}

main=$root/proj
mkdir -p "$main/crates/core/src" "$main/target/debug"
cd "$main"
git init -q
git config user.email selftest@example.invalid
git config user.name selftest
printf '[package]\nname = "core"\n' >crates/core/Cargo.toml
printf 'pub fn a() {}\n' >crates/core/src/lib.rs
printf '/target/\n' >.gitignore
echo artifact >target/debug/libcore.rlib
touch -t 202001010000 target/debug/libcore.rlib
git add -A && git commit -qm base

printf '\nprepare\n'
expect 0 "prepare makes the worktree" "$tool" prepare t1
wt=$main-wt/t1
[ -f "$wt/target/debug/libcore.rlib" ] && pass "target/ is cloned" || fail "target/ is not cloned"
[ "$(stat -f %m "$wt/target/debug/libcore.rlib")" -lt "$(stat -f %m "$wt/crates/core/src/lib.rs")" ] &&
    pass "cloned artifact is older than the checked-out source" || fail "artifact mtime is not older"
expect 1 "prepare refuses an existing task" "$tool" prepare t1
expect 1 "prepare refuses a bad task name" "$tool" prepare 'Bad/Name'

allow=$root/allow.txt
printf '# allowed\ncrates/core/src/lib.rs\ncrates/core/src/new.rs\n' >"$allow"

printf '\ngate\n'
expect 0 "no change passes" "$tool" gate t1 "$allow"
printf 'pub fn a() { let _ = 1; }\n' >"$wt/crates/core/src/lib.rs"
printf 'pub fn b() {}\n' >"$wt/crates/core/src/new.rs"
echo more >"$wt/target/debug/extra"
expect 0 "allowed edits pass; target/ is ignored" "$tool" gate t1 "$allow"

reset() { git -C "$wt" checkout -q -- . && git -C "$wt" clean -qfdx -e target; printf 'pub fn a() { let _ = 1; }\n' >"$wt/crates/core/src/lib.rs"; }

printf '[dependencies]\nserde = "1"\n' >>"$wt/crates/core/Cargo.toml"
expect 1 "Cargo.toml edit is rejected" "$tool" gate t1 "$allow"; reset
printf 'crates/core/build.rs\n' >>"$allow"; echo 'fn main(){}' >"$wt/crates/core/build.rs"
expect 1 "build.rs is rejected even when allowlisted" "$tool" gate t1 "$allow"; reset
echo x >"$wt/crates/core/src/other.rs"
expect 1 "file outside the allowlist is rejected" "$tool" gate t1 "$allow"; reset
mkdir -p "$wt/.cargo" && echo '[build]' >"$wt/.cargo/config.toml"
expect 1 ".cargo/config.toml is rejected" "$tool" gate t1 "$allow"; reset
echo junk >"$wt/crates/core/target-cache.log"; printf 'crates/core/target-cache.log\n' >>"$allow"; printf '*.log\n' >>"$wt/.gitignore"
expect 1 ".gitignore edit is rejected" "$tool" gate t1 "$allow"; reset
printf '#[cfg(test)]\nmod t { #[test] fn x() {} }\n' >>"$wt/crates/core/src/lib.rs"
expect 1 "added test module is rejected" "$tool" gate t1 "$allow"; reset
printf 'pub fn u() { unsafe {} }\n' >>"$wt/crates/core/src/lib.rs"
expect 1 "unsafe is rejected by default" "$tool" gate t1 "$allow"
expect 0 "unsafe passes with --allow-unsafe" "$tool" gate t1 "$allow" --allow-unsafe; reset
printf '#[allow(dead_code)]\nfn z() {}\n' >>"$wt/crates/core/src/lib.rs"
expect 1 "lint allow is rejected by default" "$tool" gate t1 "$allow"; reset
rm "$wt/crates/core/src/lib.rs"; [ ! -e "$wt/crates/core/src/lib.rs" ] || fail "could not delete"
expect 0 "deleting an allowlisted file passes" "$tool" gate t1 "$allow"; reset
git -C "$wt" -c user.email=a@b.invalid -c user.name=x commit -qam sneaky
expect 1 "a commit by Codex is rejected" "$tool" gate t1 "$allow"
git -C "$wt" reset -q --hard "$(cat "$main-wt/.meta/t1.base")"; reset

printf '\nintegrate + cleanup\n'
expect 0 "gate passes before integrate" "$tool" gate t1 "$allow"
expect 0 "integrate merges without a commit" "$tool" integrate t1 "feat: test change"
[ -f "$main/.git/MERGE_HEAD" ] && pass "merge is pending (no commit on main)" || fail "merge is not pending"
grep -q 'let _ = 1' crates/core/src/lib.rs && pass "change is in the main checkout" || fail "change is missing"
git merge --abort
expect 0 "cleanup removes the task" "$tool" cleanup t1
[ ! -d "$wt" ] && ! git rev-parse -q --verify codex/t1 >/dev/null && pass "worktree and branch are gone" || fail "worktree or branch remains"

echo
[ $failures = 0 ] && echo "selftest: all checks passed" || { echo "selftest: $failures failure(s)"; exit 1; }
