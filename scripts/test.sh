#!/usr/bin/env bash
# Run the test suite with every test binary inside a sandbox.
#
#   scripts/test.sh [cargo test arguments]
#
# Compiling happens outside, where cargo has its registry and its cache. Running happens
# inside, through cargo's own runner hook, which is what scripts/sandbox-run.sh is. So the
# git the tests spawn sees an empty machine: no home directory, no configuration, no agent.
#
# `nix flake check` runs the same tests in the nix build sandbox, which is isolated the
# same way. Either is a real answer; this one is the fast one.
set -euo pipefail

cd "$(dirname "$0")/.."

command -v bwrap >/dev/null || {
  echo "bwrap missing: run inside 'nix develop'" >&2
  exit 1
}

# cargo names the runner after the target it is building for.
triple="$(rustc -vV | sed -n 's/^host: //p')"
runner="CARGO_TARGET_$(echo "$triple" | tr 'a-z-' 'A-Z_')_RUNNER"

export "$runner=$PWD/scripts/sandbox-run.sh"
exec cargo test "$@"
