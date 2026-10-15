#!/usr/bin/env bash
# Populate a new staging root; never installs files on the running system.
set -euo pipefail
if (($# != 2)); then echo 'Usage: bash scripts/stage-release.sh DESTDIR RELEASE_BIN_DIR' >&2; exit 2; fi
talis_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
talis_destination=$1
talis_binaries=$2
[[ $talis_destination == /* && $talis_destination != / ]] || { echo 'DESTDIR must be an absolute staging directory, not /.' >&2; exit 2; }
[[ ! -e $talis_destination || -d $talis_destination && -z $(ls -A -- "$talis_destination") ]] || { echo 'DESTDIR must be absent or empty; existing files will not be replaced.' >&2; exit 2; }
for talis_binary in talis talis-gnome-scene; do
  [[ -x $talis_binaries/$talis_binary ]] || { echo "Missing release binary: $talis_binary" >&2; exit 1; }
done
install -d "$talis_destination"
talis_probe=$(mktemp "$talis_destination/.permissions.XXXXXX")
trap 'rm -f -- "$talis_probe"' EXIT
chmod 600 "$talis_probe"
[[ $(stat -c '%a' "$talis_probe") == 600 ]] || { echo 'Stage on a Linux/POSIX filesystem that preserves Unix permissions, not NTFS.' >&2; exit 1; }
rm -f -- "$talis_probe"
trap - EXIT
install -d "$talis_destination/usr/bin" "$talis_destination/usr/lib/talis" \
  "$talis_destination/usr/share/talis" "$talis_destination/usr/share/applications" \
  "$talis_destination/usr/share/icons/hicolor/scalable/apps" \
  "$talis_destination/usr/share/metainfo" "$talis_destination/usr/share/doc/talis"
install -m755 "$talis_binaries/talis" "$talis_destination/usr/lib/talis/talis-bin"
install -m755 "$talis_binaries/talis-gnome-scene" "$talis_destination/usr/lib/talis/talis-gnome-scene"
install -m755 "$talis_repo/scripts/launch.sh" "$talis_destination/usr/lib/talis/launch.sh"
install -m644 "$talis_repo/scripts/session.sh" "$talis_destination/usr/lib/talis/session.sh"
ln -s ../lib/talis/launch.sh "$talis_destination/usr/bin/talis"
ln -s ../lib/talis/talis-gnome-scene "$talis_destination/usr/bin/talis-gnome-scene"
# Keep an explicit runtime allowlist. Do not copy the repository or Blender tree.
cp -R -- "$talis_repo/assets" "$talis_destination/usr/share/talis/assets"
install -m644 "$talis_repo/packaging/io.github.azizstark.Talis.desktop" "$talis_destination/usr/share/applications/"
install -m644 "$talis_repo/packaging/io.github.azizstark.Talis.metainfo.xml" "$talis_destination/usr/share/metainfo/"
install -m644 "$talis_repo/assets/ui/branding/io.github.azizstark.Talis.svg" "$talis_destination/usr/share/icons/hicolor/scalable/apps/"
install -m644 "$talis_repo/LICENSE" "$talis_repo/LICENSING.md" "$talis_destination/usr/share/doc/talis/"
find "$talis_destination/usr/share" -type d -exec chmod 755 {} +
find "$talis_destination/usr/share" -type f -exec chmod 644 {} +
