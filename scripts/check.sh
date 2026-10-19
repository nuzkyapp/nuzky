#!/usr/bin/env bash
# Local gate before every release. GitHub runs the Linux checks, the dependency
# audits and the check without AVX2 on every non-draft pull request and push to main (.github/workflows/checks.yml).
# The full tests with media and models and the UI flows run here, on Linux and macOS.
# Stops at the first failing step and names it. Needs the tools listed in scripts/repro.py.
# A rerun after a failure skips the builds, clippy and Rust tests that passed for the same state; NUZKY_CHECK_FRESH=1
# starts fresh.
set -uo pipefail
cd "$(dirname "$0")/.."
os=$(uname -s)

# On macOS the gate builds against Homebrew's FFmpeg as docs/BUILDING.md describes and runs the ffmpeg command of
# ffmpeg-full, whose drawtext the engine tests draw frame numbers with.
if [ "$os" = Darwin ]; then
  export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-$(brew --prefix ffmpeg@8)/lib/pkgconfig}"
  export SDKROOT="${SDKROOT:-$(xcrun --sdk macosx --show-sdk-path)}"
  PATH="$(brew --prefix ffmpeg-full)/bin:$PATH"
  bash scripts/seed-target.sh || exit 1
fi

# One gate at a time per machine, whichever checkout it runs in: two at once take longer than one after the
# other, fail on timing checks and collide on the UI flows' ports. A second gate waits here. The lock sits with
# the build dependencies, not in XDG_RUNTIME_DIR, which a terminal running the app may have changed.
command -v flock >/dev/null || { echo "check needs flock; on macOS: brew install flock" >&2; exit 1; }
mkdir -p "$HOME/.cache/nuzky/deps"
exec 9>"$HOME/.cache/nuzky/deps/check.lock"
if ! flock -n 9; then
  echo "Another scripts/check.sh runs on this machine; this one starts when it ends."
  flock 9
fi
# The time reported at the end leaves out the wait.
SECONDS=0

resume=tmp-test/check-resume
mkdir -p "$resume"
# After a failure, a rerun skips only the expensive checks below that already passed. Guards, installs, fixtures
# and the media and UI step always run. A passed check counts again only for the same bytes of every file
# (tracked or untracked, ignored ones aside), toolchain, build variables and FFmpeg. A separate index snapshots the
# working tree without touching the real one; the empty attribute source and the two settings keep git from
# normalizing line endings.
resumable='|Frontend lint|Frontend types and build|Website build|Clippy|Rust tests|'
cp "$(git rev-parse --git-path index)" "$PWD/$resume/index"
export GIT_INDEX_FILE=$PWD/$resume/index GIT_ATTR_SOURCE=4b825dc642cb6eb9a060e54bf8d69288fbee4904
tree=$(git -c core.autocrlf=false -c core.attributesFile=/dev/null add -A && git write-tree) || exit 1
unset GIT_INDEX_FILE GIT_ATTR_SOURCE
state=$( {
  echo "$tree"; uname -sm; rustc -V; cargo -V; node -v; npm -v; ffmpeg -version | head -1; cat .cargo/config.toml
  env | LC_ALL=C sort | grep -E '^(RUST|CARGO|PKG_CONFIG|SDKROOT|PATH=|CC|CXX|CFLAGS|LDFLAGS|MACOSX_)'
} 2>&1 | git hash-object --stdin)
if [ "${NUZKY_CHECK_FRESH:-0}" = 1 ] || [ "$state" != "$(cat "$resume/state" 2>/dev/null)" ]; then
  : >"$resume/passed"
  printf '%s\n' "$state" >"$resume/state"
fi

step() {
  local name=$1 started=$SECONDS
  shift
  if [[ $resumable == *"|$name|"* ]] && grep -Fxq "$name" "$resume/passed"; then
    echo
    echo "==> $name skipped (already passed for this state)"
    return
  fi
  echo
  echo "==> $name"
  # The steps do not inherit the lock, so a process one of them leaves behind cannot hold it.
  if ! "$@" 9>&-; then
    echo
    echo "check FAILED at: $name ($((SECONDS - started))s)" >&2
    exit 1
  fi
  printf '%s\n' "$name" >>"$resume/passed"
  echo "<== $name passed ($((SECONDS - started))s)"
}

# npm ci empties node_modules, which must not be another checkout's directory behind a symlink.
own_node_modules() {
  if [ -L node_modules ]; then
    echo "node_modules is a symlink to another checkout. Remove the link and run the check again." >&2
    return 1
  fi
}

# Tests must not touch the user's Nuzky transcripts, caches or the socket of their running app.
runtime=$(mktemp -d "${TMPDIR:-/tmp}/nuzky-check.XXXXXX")
trap 'rm -rf "$runtime"' EXIT
isolated() {
  local data=$PWD/tmp-test/xdg/data cache=$PWD/tmp-test/xdg/cache
  if [ "$os" = Darwin ]; then
    # macOS ignores the XDG variables and keeps Nuzky's data and cache under ~/Library, so the tests get a home
    # of their own whose Library folders lead to the same directories. Cargo, rustup and the build dependencies
    # stay in the real home.
    local home=$PWD/tmp-test/xdg/home
    mkdir -p "$home/Library/Application Support" "$home/Library/Caches" "$data/nuzky" "$cache/nuzky"
    ln -sfn "$data/nuzky" "$home/Library/Application Support/nuzky"
    ln -sfn "$cache/nuzky" "$home/Library/Caches/nuzky"
    set -- HOME="$home" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
      NUZKY_DEPS="${NUZKY_DEPS:-$HOME/.cache/nuzky/deps}" "$@"
  fi
  env -u NUZKY_DATA_DIR XDG_DATA_HOME="$data" XDG_CACHE_HOME="$cache" XDG_RUNTIME_DIR="$runtime" "$@"
}

# The Rust tests with media and the UI flows spend their time waiting for speech recognition on the GPU, the app
# and real time, each with about a tenth of the CPU, so they run side by side. The flows' report follows the tests'.
media_tests_and_flows() {
  local flows=tmp-test/ui-flows.log tests_failed=0 flows_failed=0
  python3 scripts/repro.py --all >"$flows" 2>&1 &
  local pid=$!
  # A job in the background ignores Ctrl+C, so a stopped gate stops its UI flows too and waits for their cleanup.
  trap 'kill -TERM "$pid" 2>/dev/null; wait "$pid"; exit 130' INT
  trap 'kill -TERM "$pid" 2>/dev/null; wait "$pid"; exit 143' TERM
  isolated cargo test --workspace --locked -- --ignored || tests_failed=1
  wait "$pid" || flows_failed=1
  trap - INT TERM
  echo
  cat "$flows"
  [ "$tests_failed" = 0 ] || echo "Rust tests with media and models FAILED" >&2
  [ "$flows_failed" = 0 ] || echo "UI flows FAILED" >&2
  [ "$tests_failed$flows_failed" = 00 ]
}

step "Own node_modules" own_node_modules
step "npm install from the lockfile" npm ci --no-audit --no-fund
step "Rust format" cargo fmt --all --check
step "Frontend lint" npm run lint
step "Script tests" node --test 'scripts/*.test.mjs'
step "Frontend types and build" npm run build
step "Website build" npm run site:build
step "Clippy" cargo clippy --workspace --all-targets --locked -- -D warnings
step "ONNX Runtime" node scripts/fetch-onnxruntime.mjs
step "Rust tests" isolated cargo test --workspace --locked
step "Test media and models" scripts/fixtures.sh
step "Rust tests with media and models, and the UI flows" media_tests_and_flows

: >"$resume/passed"
echo
echo "check passed in $((SECONDS / 60))m $((SECONDS % 60))s"
