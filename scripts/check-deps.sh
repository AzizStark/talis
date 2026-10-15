#!/usr/bin/env bash
# Check only; never installs packages or changes the system.
set -euo pipefail
case "${1:-x11}" in
  x11|gnome) talis_wayland=false ;;
  wayland) talis_wayland=true ;;
  *) echo 'Usage: bash scripts/check-deps.sh [x11|wayland|gnome]' >&2; exit 2 ;;
esac
if (($# > 1)); then exit 2; fi
for talis_tool in cargo rustc pkg-config; do
  command -v "$talis_tool" >/dev/null || { echo "Missing build tool: $talis_tool" >&2; exit 1; }
done
if ! pkg-config --atleast-version=4.8 gtk4; then
  echo 'GTK4 development packages (4.8+) are required.' >&2
  echo 'Debian/Ubuntu/Mint: build-essential pkg-config libgtk-4-dev' >&2
  echo 'Fedora: gcc pkgconf-pkg-config gtk4-devel' >&2
  echo 'Arch/CachyOS: base-devel pkgconf gtk4' >&2
  exit 1
fi
if $talis_wayland && ! pkg-config --exists gtk4-layer-shell-0; then
  echo 'Wayland builds additionally require GTK4 layer-shell development packages.' >&2
  echo 'Fedora: gtk4-layer-shell-devel; Arch/CachyOS: gtk4-layer-shell' >&2
  exit 1
fi
command -v curl >/dev/null || echo 'Optional curl is missing; remote artwork will be unavailable.' >&2
echo "Build dependencies ready (GTK $(pkg-config --modversion gtk4))."
