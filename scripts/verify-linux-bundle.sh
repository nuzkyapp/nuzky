#!/usr/bin/env bash
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
target="${CARGO_TARGET_DIR:-$repo/target}"
target="$(cd "$target" && pwd)"
binary="$target/release/nuzky-app"
ldd "$binary" | tee "$target/release/ldd.txt"
if grep -q 'not found' "$target/release/ldd.txt"; then
  echo 'Unresolved runtime dependency' >&2
  exit 1
fi
shopt -s nullglob

# Exactly one package of a kind, or the check would pass on an old build left beside the new one.
one() {
  local kind="$1"
  shift
  if [ "$#" -ne 1 ]; then
    echo "Expected one $kind in $target/release/bundle, found $#; remove the old ones and build again" >&2
    exit 1
  fi
}

# scripts/build-bundles.mjs builds the AppImage and the .deb on Debian and Ubuntu, which the release needs, and an .rpm
# on other hosts with rpm; Fedora also builds an AppImage by hand (docs/BUILDING.md). Each bundle found is checked.
# shellcheck disable=SC1091
os_ids="$( . /etc/os-release 2>/dev/null && echo "${ID:-} ${ID_LIKE:-}" || echo debian)"
debian_like=false
if grep -Ewq 'debian|ubuntu' <<<"$os_ids"; then debian_like=true; fi
rpms=("$target"/release/bundle/rpm/*.rpm)
images=("$target"/release/bundle/appimage/*.AppImage)
debs=("$target"/release/bundle/deb/*.deb)
if ! $debian_like && [ "${#rpms[@]}" -eq 0 ] && [ "${#images[@]}" -eq 0 ] && [ "${#debs[@]}" -eq 0 ]; then
  echo "No .rpm, AppImage or .deb in $target/release/bundle" >&2
  exit 1
fi

verify_rpm() {
  one .rpm "${rpms[@]}"
  rpm -qlpv "${rpms[0]}" >"$target/release/rpm-contents.txt"
  # A regular file, not a link.
  if ! grep -Eq '^-.* /usr/lib/Nuzky/libonnxruntime\.so\.1$' "$target/release/rpm-contents.txt"; then
    echo 'ONNX Runtime missing from the rpm' >&2
    exit 1
  fi
  if ! grep -Eq '^-.* /usr/bin/nuzky-app$' "$target/release/rpm-contents.txt"; then
    echo 'nuzky-app missing from the rpm' >&2
    exit 1
  fi
  rpm -qpR "${rpms[0]}" | tee "$target/release/rpm-requires.txt"
  # Every library the binary links must be a requirement, or the package installs where it cannot start.
  for soname in $(objdump -p "$binary" | awk '/NEEDED/ {print $2}'); do
    case "$soname" in
      libav*|libsw*|libwebkit2gtk*|libgtk-3*|libasound*)
        grep -Fxq "$soname()(64bit)" "$target/release/rpm-requires.txt" || {
          echo "The rpm does not require $soname, which the binary links" >&2
          exit 1
        } ;;
    esac
  done
  # wgpu and Whisper open Vulkan at run time, so the binary does not link it.
  if ! grep -Fxq 'libvulkan.so.1()(64bit)' "$target/release/rpm-requires.txt"; then
    echo 'The rpm does not require libvulkan.so.1' >&2
    exit 1
  fi
  # Every requirement must be installable here.
  while read -r requirement; do
    case "$requirement" in rpmlib\(*|/*) continue ;; esac
    rpm -q --whatprovides "$requirement" >/dev/null || {
      echo "Nothing on this system provides $requirement" >&2
      exit 1
    }
  done <"$target/release/rpm-requires.txt"
  stat --printf='%n: %s bytes\n' "${rpms[0]}"
}

verify_appimage() {
  one AppImage "${images[@]}"
  extract="$(mktemp -d)"
  trap 'rm -rf "$extract"' EXIT
  (cd "$extract" && "${images[0]}" --appimage-extract >/dev/null)
  test -x "$extract/squashfs-root/AppRun"
  app_binary="$extract/squashfs-root/usr/bin/nuzky-app"
  test -x "$app_binary"
  libs="$(find "$extract/squashfs-root/usr" -type d -name 'lib*' -print | paste -sd :)"
  LD_LIBRARY_PATH="$libs" ldd "$app_binary" | tee "$target/release/appimage-ldd.txt"
  if grep -q 'not found' "$target/release/appimage-ldd.txt"; then
    echo 'Unresolved extracted AppImage dependency' >&2
    exit 1
  fi
  for lib in avcodec avformat avutil swscale swresample; do
    find "$extract/squashfs-root" -name "lib$lib.so.*" -print -quit | grep -q .
  done
  # ONNX Runtime, which the app opens from its resource directory when covers run.
  ort="$extract/squashfs-root/usr/lib/Nuzky/libonnxruntime.so.1"
  test -f "$ort"
  ldd "$ort" | tee "$target/release/onnxruntime-ldd.txt"
  if grep -q 'not found' "$target/release/onnxruntime-ldd.txt"; then
    echo 'Unresolved ONNX Runtime dependency' >&2
    exit 1
  fi
  stat --printf='%n: %s bytes\n' "${images[0]}"
}

verify_deb() {
  one .deb "${debs[@]}"
  # A regular file, not a link; Tauri writes the deb's paths without a leading ./
  dpkg-deb -c "${debs[0]}" >"$target/release/deb-contents.txt"
  if ! grep -Eq '^-.* (\./)?usr/lib/Nuzky/libonnxruntime\.so\.1$' "$target/release/deb-contents.txt"; then
    echo 'ONNX Runtime missing from the deb' >&2
    exit 1
  fi
  stat --printf='%n: %s bytes\n' "${debs[0]}"
}

if [ "${#rpms[@]}" -gt 0 ]; then verify_rpm; fi
if $debian_like || [ "${#images[@]}" -gt 0 ]; then verify_appimage; fi
if $debian_like || [ "${#debs[@]}" -gt 0 ]; then verify_deb; fi
stat --printf='%n: %s bytes\n' "$binary"
