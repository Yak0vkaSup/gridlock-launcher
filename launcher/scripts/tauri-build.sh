#!/usr/bin/env bash
# Wrapper around `tauri build`, used by .github/workflows/launcher.yml (tauri-action's tauriScript).
#
# After the build, every Linux AppImage is repacked WITHOUT the bundled libwayland-* libraries and
# re-signed. linuxdeploy copies the build machine's libwayland (Ubuntu 22.04, 1.20) into the AppImage;
# on a host with a newer Mesa the mismatch kills WebKit at start-up with
#   "Could not create default EGL display: EGL_BAD_PARAMETER. Aborting..."
# libwayland is a base library on every distro that can run WebKitGTK, so the host copy is the one
# to use (same reasoning as libEGL/libGL, which linuxdeploy already leaves out).
set -euo pipefail
cd "$(dirname "$0")/.."

npx tauri "$@"

[ "$(uname -s)" = Linux ] || exit 0
case " $* " in *" build "*) ;; *) exit 0 ;; esac

shopt -s nullglob
for img in src-tauri/target/release/bundle/appimage/*.AppImage; do
  work=$(mktemp -d)
  chmod +x "$img"
  offset=$("$img" --appimage-offset)
  (cd "$work" && "$OLDPWD/$img" --appimage-extract >/dev/null)
  removed=$(ls "$work"/squashfs-root/usr/lib/libwayland-* 2>/dev/null | wc -l)
  rm -f "$work"/squashfs-root/usr/lib/libwayland-*
  head -c "$offset" "$img" >"$work/runtime"
  mksquashfs "$work/squashfs-root" "$work/fs.squashfs" -root-owned -noappend -comp zstd -Xcompression-level 19 -no-progress >/dev/null
  cat "$work/runtime" "$work/fs.squashfs" >"$img"
  rm -rf "$work"
  if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    npx tauri signer sign "$img"
  fi
  echo "tauri-build.sh: repacked $img without $removed bundled libwayland libraries"
done
