# Talis portable

Extract into a permanent folder, open a terminal there, and run:

```sh
./talis
```

## GNOME Wayland

Run this once before launching Talis:

```sh
bash setup-gnome.sh
```

Then run `./talis` and follow the setup prompt. Log out/in if asked.
Setup adds a menu entry; enabling the companion also starts Talis at sign-in.
This integration currently supports one monitor.

## Hyprland

Talis appears below bar decorations by default. If your wallpaper covers Talis,
quit Talis and try:

```sh
TALIS_WAYLAND_LAYER=bottom ./talis
```

To return to the default, quit Talis and launch `./talis` normally.

## Requirements

x86-64 Linux, glibc 2.39+, GTK 4.8+ with SVG support, and OpenGL 3.3.
GTK and graphics drivers come from your system. Remote cover art needs `curl`
and CA certificates.

Keep the folder in place so startup and GNOME integration continue working.
Quit Talis before updating; saved settings are kept separately.

Licenses and credits: `usr/share/doc/talis/`.
