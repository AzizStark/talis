#!/usr/bin/env bash
# Stage both desktop backends. The supplied library must match the Wayland build.
set -euo pipefail
if (($# != 5)); then
  echo 'Usage: bash scripts/bundle.sh DESTDIR X11_BIN_DIR WAYLAND_BINARY LAYER_LIBRARY LAYER_LICENSE' >&2
  exit 2
fi
talis_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
talis_destination=$1
[[ -x $3 && -f $4 && -f $5 ]] || { echo 'Missing Wayland binary, layer-shell library or its license.' >&2; exit 1; }
# The builder supplies trusted compiled inputs; never infer an unrelated SDK.
for talis_binary in "$2/talis" "$2/talis-gnome-scene" "$3" "$4"; do
  [[ $(LC_ALL=C readelf -h "$talis_binary") == *'Advanced Micro Devices X86-64'* ]] || {
    echo "Not an x86-64 ELF input: $talis_binary" >&2; exit 1;
  }
done
for talis_binary in "$2/talis" "$2/talis-gnome-scene"; do
  if readelf -d "$talis_binary" | grep -Fq '[libgtk4-layer-shell.so.0]'; then
    echo 'X11 and GNOME client inputs must be built without wayland-desktop.' >&2; exit 1
  fi
done
readelf -d "$3" | grep -Fq '[libgtk4-layer-shell.so.0]' || { echo 'Wayland binary does not link GTK4 layer-shell.' >&2; exit 1; }
readelf -d "$4" | grep -Fq '[libgtk4-layer-shell.so.0]' || { echo 'Unexpected layer-shell library SONAME.' >&2; exit 1; }
bash "$talis_repo/scripts/stage-release.sh" "$talis_destination" "$2"
install -m755 "$3" "$talis_destination/usr/lib/talis/talis-wayland-bin"
install -m644 "$4" "$talis_destination/usr/lib/talis/libgtk4-layer-shell.so.0"
install -m644 "$5" "$talis_destination/usr/share/doc/talis/gtk4-layer-shell-LICENSE"
install -d "$talis_destination/usr/share/talis/gnome"
install -m644 "$talis_repo"/integrations/gnome/*.js "$talis_repo"/integrations/gnome/*.json "$talis_destination/usr/share/talis/gnome/"
install -m644 "$talis_repo/LICENSE" "$talis_destination/usr/share/talis/gnome/LICENSE"
install -m755 "$talis_repo/packaging/setup-gnome.sh" "$talis_destination/setup-gnome.sh"
install -m644 "$talis_repo/packaging/PORTABLE.md" "$talis_destination/README.md"
ln -s usr/bin/talis "$talis_destination/talis"
printf 'Portable runtime staged: %s\n' "$talis_destination"
