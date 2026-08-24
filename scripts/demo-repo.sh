#!/usr/bin/env bash
# Build the repository the interface captures are taken against.
#
#   scripts/demo-repo.sh [directory]
#
# Six commits, a tag, two remotes with different amounts of history on them, and one commit
# on neither. Every git here runs inside the same sandbox the tests use, so building it
# cannot read the machine's configuration or reach a key.
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
out="${1:-$root/.tmp/demo}"

command -v bwrap >/dev/null || {
  echo "bwrap missing: run inside 'nix develop'" >&2
  exit 1
}

rm -rf "$out"
mkdir -p "$out"

# The build directory is not needed here, so only the store and the output are bound. No
# /home exists inside, and git has nowhere to read a configuration or a key from.
bwrap \
  --ro-bind /nix/store /nix/store \
  --bind "$out" /demo \
  --proc /proc \
  --dev /dev \
  --tmpfs /tmp \
  --dir /tmp/home \
  --setenv HOME /tmp/home \
  --setenv TMPDIR /tmp \
  --unsetenv SSH_AUTH_SOCK \
  --unsetenv GPG_AGENT_INFO \
  --unshare-all \
  --new-session \
  --die-with-parent \
  --chdir /demo \
  -- sh -euc '
    git config --global user.name "gg demo"
    git config --global user.email "demo@test.invalid"
    git config --global commit.gpgsign false
    git config --global init.defaultBranch master

    git init --quiet repo
    git init --quiet --bare origin.git
    git init --quiet --bare backup.git

    cd repo
    for n in 1 2 3 4 5; do
      echo "line $n" >> notes.md
      git add notes.md
      git commit --quiet -m "note $n" -m "a body for note $n"
    done

    # Relative, so the remotes still resolve wherever this tree is mounted. An absolute
    # path here would name a directory that only exists inside this sandbox.
    git remote add origin ../origin.git
    git remote add backup ../backup.git
    git push --quiet origin master

    echo unpushed >> notes.md
    git add notes.md
    git commit --quiet -m "not on any remote"
    git tag v0.1.0

    # One remote and nothing else, which is what a push has to ask about too.
    cd /demo
    git init --quiet solo
    git init --quiet --bare solo-origin.git
    cd solo
    echo alone > readme.md
    git add readme.md
    git commit --quiet -m "first"
    git remote add origin ../solo-origin.git
  '

echo "built $out/repo with remotes origin and backup"
