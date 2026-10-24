#!/usr/bin/env bash
# Starts the build of a new checkout from the build of another worktree of this repository, so its first build
# compiles only the Nuzky crates instead of every dependency (whisper.cpp, the FFmpeg bindings, wgpu, Tauri).
# Linux setup and the macOS build scripts run it. It does nothing when target/ already holds most of a build
# or no other worktree has one. Linux needs btrfs or XFS reflinks; macOS uses APFS clones.
#
# The copy shares disk blocks with the original until either changes. Nothing of the workspace's own crates is
# reused: cargo names their builds the same in every checkout, so with a copied fingerprint it would run the
# other worktree's code. Their fingerprints are removed before anything is copied, and none is copied over.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(uname -s)" = Darwin ]; then
  clone=-c
  stat_time=(-f %m)
  stat_device=(-f %d)
  hash=(shasum -a 256)
else
  clone=--reflink=always
  stat_time=(-c %Y)
  stat_device=(-c %d)
  hash=(sha256sum)
fi

fingerprints() { find "$1/target/debug/.fingerprint" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l; }
same_inputs() { cat "$1/Cargo.lock" "$1/Cargo.toml" 2>/dev/null | "${hash[@]}" | cut -d' ' -f1; }
here=$(pwd -P)
ours=$(same_inputs .)

# The most recently built worktree, preferring one with the same Cargo.lock and profiles.
donor='' best=''
while read -r key path; do
  [ "$key" = worktree ] && [ -d "$path/target/debug/.fingerprint" ] || continue
  [ "$(cd "$path" && pwd -P)" = "$here" ] && continue
  # Across file systems macOS `cp -c` quietly copies every byte instead of cloning.
  [ "$(stat "${stat_device[@]}" "$path/target/debug/deps")" = "$(stat "${stat_device[@]}" .)" ] || continue
  rank="$([ "$(same_inputs "$path")" = "$ours" ] && echo 1 || echo 0)$(stat "${stat_time[@]}" "$path/target/debug/deps")"
  if [[ $rank > $best ]]; then best=$rank donor=$path; fi
done < <(git worktree list --porcelain)
[ -n "$donor" ] || exit 0
[ "$(fingerprints .)" -lt $(($(fingerprints "$donor") / 2)) ] || exit 0

mkdir -p target/debug/.fingerprint
exec 8>target/debug/.cargo-build-lock
flock -n 8 || { echo "cargo is building in this checkout; not starting from another build" >&2; exit 0; }
probe=target/debug/.reflink-probe
if ! cp "$clone" Cargo.lock "$probe" 2>/dev/null; then
  echo "This file system cannot clone files; the first build starts from scratch." >&2
  exit 0
fi
rm -f "$probe"

echo "Starting target/ from the build in $donor"
workspace=$(cargo metadata --format-version 1 --no-deps --offline |
  python3 -c 'import json, sys; print("\n".join(p["name"] for p in json.load(sys.stdin)["packages"]))')
own() { while read -r name; do find "$1" -mindepth 1 -maxdepth 1 -name "$name-*"; done <<<"$workspace"; }
own target/debug/.fingerprint | while IFS= read -r path; do rm -rf "$path"; done
from=$donor/target/debug
# Cargo holds this lock while it builds there, so nothing is copied half written.
exec 7<"$from/.cargo-build-lock"
flock -s -w 300 7 || { echo "cargo is still building in $donor; the first build starts from scratch." >&2; exit 0; }
cp -a "$clone" "$from/build" "$from/deps" target/debug/
workspace_pattern="^($(paste -sd'|' - <<<"$workspace"))-"
for fingerprint in "$from/.fingerprint"/*; do
  [[ ${fingerprint##*/} =~ $workspace_pattern ]] && continue
  cp -a "$clone" "$fingerprint" target/debug/.fingerprint/
done
left=$(own target/debug/.fingerprint)
[ -z "$left" ] || { echo "seed-target: a workspace fingerprint was copied: $left" >&2; rm -rf $left; exit 1; }
# Cargo moves the paths in a build script's `output` along with the build, but not those in files the script
# wrote, such as Tauri's lists of permission files. Those would name the build they were made in, and this one
# would stop building once that is deleted. The copies name this build instead and keep the times cargo compares.
for run in target/debug/build/*/root-output; do
  old=$(cat "$run")
  old=${old%/debug/build/*}/
  [ "$old" = "$here/target/" ] && continue
  files=$(grep -rlIF -- "$old" "${run%/*}") || continue
  OLD=$old NEW=$here/target/ python3 -c '
import os, sys
for path in sys.stdin.read().splitlines():
    times = os.stat(path)
    with open(path, "rb") as f:
        text = f.read()
    with open(path, "wb") as f:
        f.write(text.replace(os.environb[b"OLD"], os.environb[b"NEW"]))
    os.utime(path, ns=(times.st_atime_ns, times.st_mtime_ns))' <<<"$files"
done
