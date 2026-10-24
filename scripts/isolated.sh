#!/usr/bin/env bash
# Runs a command, such as `cargo test --workspace <name> -- --ignored`, with Nuzky's data and cache in tmp-test/xdg
# and a socket folder of its own. Tests must not touch the user's Nuzky transcripts, caches or the socket of their
# running app. scripts/check.sh runs the Rust tests through it.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
data=$root/tmp-test/xdg/data cache=$root/tmp-test/xdg/cache
runtime=$(mktemp -d "${TMPDIR:-/tmp}/nuzky-check.XXXXXX")
trap 'rm -rf "$runtime"' EXIT
if [ "$(uname -s)" = Darwin ]; then
  # macOS ignores the XDG variables and keeps Nuzky's data and cache under ~/Library, so the tests get a home
  # of their own whose Library folders lead to the same directories. Cargo, rustup and the build dependencies
  # stay in the real home.
  home=$root/tmp-test/xdg/home
  mkdir -p "$home/Library/Application Support" "$home/Library/Caches" "$data/nuzky" "$cache/nuzky"
  ln -sfn "$data/nuzky" "$home/Library/Application Support/nuzky"
  ln -sfn "$cache/nuzky" "$home/Library/Caches/nuzky"
  set -- HOME="$home" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
    NUZKY_DEPS="${NUZKY_DEPS:-$HOME/.cache/nuzky/deps}" "$@"
fi
env -u NUZKY_DATA_DIR XDG_DATA_HOME="$data" XDG_CACHE_HOME="$cache" XDG_RUNTIME_DIR="$runtime" "$@"
