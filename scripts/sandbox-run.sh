#!/usr/bin/env bash
# Run one test binary with no view of the machine it is running on.
#
# This is cargo's test runner, set by scripts/test.sh. cargo hands it the absolute path of
# a binary under ./target; the sandbox mounts that build directory at /build and runs the
# binary from there, so no path from the developer's home exists inside, not even as an
# empty directory leading to a mount point.
#
# The tests in src/git/command.rs spawn real git. What must not happen is git finding a
# configuration, a signing key or an agent. Nothing that could carry one is bound: there is
# no /home, no /etc, no /run, and the home directory is an empty tmpfs. The store is
# read-only and holds git and every library the binary links. Everything the run writes
# goes to a tmpfs discarded when it exits.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
binary="$1"
shift

case "$binary" in
  "$root"/target/*) inside="/build/${binary#"$root"/target/}" ;;
  *)
    echo "sandbox-run.sh takes a binary under $root/target, got $binary" >&2
    exit 1
    ;;
esac

exec bwrap \
  --ro-bind /nix/store /nix/store \
  --ro-bind "$root/target" /build \
  --proc /proc \
  --dev /dev \
  --tmpfs /tmp \
  --dir /tmp/home \
  --setenv HOME /tmp/home \
  --setenv TMPDIR /tmp \
  --unsetenv SSH_AUTH_SOCK \
  --unsetenv GPG_AGENT_INFO \
  --unsetenv XDG_RUNTIME_DIR \
  --unshare-all \
  --new-session \
  --die-with-parent \
  --chdir /tmp \
  -- "$inside" "$@"
