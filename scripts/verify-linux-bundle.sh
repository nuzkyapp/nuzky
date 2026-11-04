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

# A host that is not Debian or Ubuntu but has rpm builds only the .rpm (scripts/build-bundles.mjs).
# shellcheck disable=SC1091
os_ids="$( . /etc/os-release 2>/dev/null && echo "${ID:-} ${ID_LIKE:-}" || echo debian)"
if command -v rpm >/dev/null && ! grep -Ewq 'debian|ubuntu' <<<"$os_ids"; then
  rpms=("$target"/release/bundle/rpm/*.rpm)
  test "${#rpms[@]}" -eq 1
  rpm -qlpv "${rpms[0]}" >"$target/release/rpm-contents.txt"
  # A regular file, not a link.
  if ! grep -Eq '^-.* /usr/lib/Nuzky/libonnxruntime\.so\.1$' "$target/release/rpm-contents.txt"; then
    echo 'ONNX Runtime missing from the rpm' >&2
    exit 1
  fi
  grep -Eq '^-.* /usr/bin/nuzky-app$' "$target/release/rpm-contents.txt"
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
  grep -Fxq 'libvulkan.so.1()(64bit)' "$target/release/rpm-requires.txt"
  # Every requirement must be installable here.
  while read -r requirement; do
    case "$requirement" in rpmlib\(*|/*) continue ;; esac
    rpm -q --whatprovides "$requirement" >/dev/null || {
      echo "Nothing on this system provides $requirement" >&2
      exit 1
    }
  done <"$target/release/rpm-requires.txt"
  stat --printf='%n: %s bytes\n' "${rpms[0]}" "$binary"
  exit 0
fi

images=("$target"/release/bundle/appimage/*.AppImage)
test "${#images[@]}" -eq 1
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
debs=("$target"/release/bundle/deb/*.deb)
test "${#debs[@]}" -eq 1
# A regular file, not a link; Tauri writes the deb's paths without a leading ./
dpkg-deb -c "${debs[0]}" >"$target/release/deb-contents.txt"
if ! grep -Eq '^-.* (\./)?usr/lib/Nuzky/libonnxruntime\.so\.1$' "$target/release/deb-contents.txt"; then
  echo 'ONNX Runtime missing from the deb' >&2
  exit 1
fi
stat --printf='%n: %s bytes\n' "${images[0]}" "$binary"
