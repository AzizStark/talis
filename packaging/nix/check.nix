{
  runCommand,
  talis,
  desktop-file-utils,
}:

runCommand "talis-nix-package-check"
  {
    nativeBuildInputs = [ desktop-file-utils ];
  }
  ''
    export HOME="$TMPDIR/home"
    mkdir -p "$HOME"
    export XDG_CURRENT_DESKTOP=KDE
    TALIS_BACKEND=x11 ${talis}/bin/talis --help > x11-help
    TALIS_BACKEND=wayland ${talis}/bin/talis --help > wayland-help
    ${talis}/bin/talis-gnome-scene --help > gnome-help
    grep -q 'Usage:' x11-help
    grep -q 'Usage:' wayland-help
    grep -q 'Usage:' gnome-help
    desktop-file-validate ${talis}/share/applications/io.github.azizstark.Talis.desktop
    for resolution in 1080p 1440p 4k; do
      for scene in day night; do
        test -s ${talis}/share/talis/assets/scenes/$resolution/$scene/beauty.png
      done
    done
    extension=${talis}/share/gnome-shell/extensions/talis@azizstark
    test -x "$extension/talis-gnome-scene"
    test -s "$extension/assets/scenes/1440p/day/beauty.png"
    test -s "$extension/extension.js"
    test -s ${talis}/share/icons/hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg
    test -s ${talis}/share/doc/talis/LICENSING.md
    touch "$out"
  ''
