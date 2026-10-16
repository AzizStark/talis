# Talis portable Linux runtime

Extract into a permanent directory, then run `./talis`.
No compiler, root installation, or backend selection is needed.

- x86-64 Linux with glibc 2.39+, GTK 4.8+ (including its SVG loader), and OpenGL 3.3.
- X11: Cinnamon, MATE, Xfce and other supported X11 hosts.
- Wayland: Plasma, Sway and Hyprland. GTK4 layer-shell is included.
- GNOME Wayland: one monitor; Shell 48/50 tested, other versions may attempt startup.
  Fresh setup tested on
  Ubuntu 26.04.1 / GNOME 50.1 with desktop icons enabled.
  Run `bash setup-gnome.sh` once and
  launch `./talis` to finish setup. The enabled companion starts Talis at login;
  `./talis` and the Talis menu entry open its Settings, not a second renderer.
- `curl` and CA certificates are required for remote artwork.

GTK, graphics drivers and base system libraries are not bundled. On CachyOS:
`sudo pacman -S --needed gtk4 librsvg curl ca-certificates`.
This is a test bundle, not an AppImage or a claim to support every Linux release.

Preferences stay in your normal XDG configuration directory. Close Talis before
switching builds. Keep the extracted directory in place; if moved, re-enable
login startup and repeat GNOME companion setup where applicable. Nothing is
added to your menu or login startup just by extracting or running the bundle.
GNOME setup explicitly adds a per-user menu entry; remove
`~/.local/share/applications/io.github.azizstark.Talis.desktop` to undo it
(use your `XDG_DATA_HOME` instead of `~/.local/share` if customized).

Application and asset notices: `usr/share/doc/talis/`.
