#!/usr/bin/env bash
# Explicit, per-user setup only. Never enables an extension or runs as root.
set -euo pipefail
[[ $EUID != 0 ]] || { echo 'Run this as your desktop user, not root.' >&2; exit 1; }
talis_bundle=$(cd -- "$(dirname -- "$(readlink -f -- "${BASH_SOURCE[0]}")")" && pwd)
command -v gnome-shell >/dev/null || { echo 'GNOME Shell is not installed.' >&2; exit 1; }
talis_data=${XDG_DATA_HOME:-${HOME:?}/.local/share}
[[ $talis_data == /* ]] || { echo 'XDG_DATA_HOME must be absolute.' >&2; exit 1; }
talis_extensions=$talis_data/gnome-shell/extensions
talis_extension=$talis_extensions/talis@azizstark
[[ ! -e $talis_extension && ! -L $talis_extension ]] || {
  echo "An extension already exists at $talis_extension; disable and back it up before replacing it." >&2; exit 1;
}
# GNOME resolves a Wayland window's application ID through its desktop file.
# Use the bundle's own icon, without modifying the host's icon theme or dock.
talis_applications=$talis_data/applications
talis_desktop=$talis_applications/io.github.azizstark.Talis.desktop
[[ ! -e $talis_desktop && ! -L $talis_desktop ]] || {
  echo "An application entry already exists at $talis_desktop; back it up before replacing it." >&2; exit 1;
}
talis_icon=$talis_bundle/usr/share/talis/assets/ui/branding/io.github.azizstark.Talis.svg
[[ -f $talis_icon && -x $talis_bundle/talis ]] || { echo 'The portable launcher or icon is missing.' >&2; exit 1; }
mkdir -p -- "$talis_extensions"
talis_temporary=$(mktemp -d "$talis_extensions/.talis-setup.XXXXXX")
talis_desktop_temporary=
trap 'rm -rf -- "$talis_temporary"; if [[ -n $talis_desktop_temporary ]]; then rm -f -- "$talis_desktop_temporary"; fi' EXIT
install -m644 "$talis_bundle"/usr/share/talis/gnome/* "$talis_temporary/"
ln -s "$talis_bundle/usr/share/talis/assets" "$talis_temporary/assets"
ln -s "$talis_bundle/usr/bin/talis-gnome-scene" "$talis_temporary/talis-gnome-scene"
# Exec is a desktop-entry command, not a shell command. Escape its quoted
# argument first, then escape the desktop string itself (including newlines).
talis_exec=$talis_bundle/talis
talis_exec=${talis_exec//\\/\\\\}
talis_exec=${talis_exec//\"/\\\"}
talis_exec=${talis_exec//\$/\\$}
talis_exec=${talis_exec//\`/\\\`}
talis_exec=${talis_exec//%/%%}
talis_exec=\"$talis_exec\"
desktop_string() {
  local value=$1
  value=${value//\\/\\\\}
  value=${value//$'\n'/\\n}
  value=${value//$'\r'/\\r}
  value=${value//$'\t'/\\t}
  printf '%s' "$value"
}
mkdir -p -- "$talis_applications"
talis_desktop_temporary=$(mktemp "$talis_applications/.talis-setup.XXXXXX")
{
  printf '[Desktop Entry]\nType=Application\nName=Talis\nComment=A music-aware desktop turntable\n'
  # env keeps the executable token unquoted for GLib's application lookup;
  # the relocated bundle path remains a correctly escaped, single argument.
  printf 'Exec=/usr/bin/env %s\nIcon=%s\n' "$(desktop_string "$talis_exec")" "$(desktop_string "$talis_icon")"
  printf 'StartupWMClass=io.github.azizstark.Talis\nTerminal=false\nStartupNotify=false\nCategories=AudioVideo;Audio;\n'
} > "$talis_desktop_temporary"
chmod 644 "$talis_desktop_temporary"
# -T prevents nesting in a directory created by another setup process.
mv -T -n -- "$talis_temporary" "$talis_extension"
[[ ! -d $talis_temporary ]] || { echo 'Another setup created the extension; nothing replaced.' >&2; exit 1; }
mv -T -n -- "$talis_desktop_temporary" "$talis_desktop"
[[ ! -f $talis_desktop_temporary ]] || { echo 'Another setup created the menu entry; nothing replaced.' >&2; exit 1; }
trap - EXIT
echo 'Installed the companion without enabling it. Keep this bundle in place.'
echo 'Added the Talis menu/dock entry. The enabled extension manages login startup.'
echo 'Launch ./talis to finish setup, then follow its logout/login instructions.'
