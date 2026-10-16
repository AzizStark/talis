# Talis

An interactive, music-aware desktop turntable that brings the classic vinyl
experience to your Linux desktop.

Watch records spin and the tonearm glide in real time as your music plays—and
control your playback directly from the turntable.

[Install](#install) · [First launch](#first-launch) ·
[Compatibility](#desktop-compatibility) · [Get help](#help-and-updates)

## Features

- 🎶 **Music-Reactive Animations:** Real-time vinyl rotation and tonearm movement
  synced with your audio playback.

- 🎛️ **Interactive Controls:** Play, pause, skip, and manage playback directly
  from the desktop interface.

- 🌓 **Day & Night Themes:** Easily switch between light and dark visual styles
  to match your desktop setup.

- 🖥️ **Hi-Res Graphics:** Rendered smoothly with high-resolution assets designed
  for displays up to 4K.

Available options depend on your [desktop integration](#desktop-compatibility).

## Install

Use the prebuilt package for your distribution. You do **not** need Rust,
Cargo, a compiler or Blender to install and use it.

Current prebuilt packages are for x86-64 Linux.

Run the commands below from the folder containing your downloaded package,
replacing the example filename with its actual name.

### Ubuntu, Linux Mint and Debian-based distributions

For compatible Debian-based systems, install the `.deb` package:

```sh
sudo apt install ./talis_VERSION_amd64.deb
```

APT installs the required runtime libraries. Open **Talis** from your
application menu when installation finishes.

### Arch Linux and CachyOS

```sh
sudo pacman -U ./talis-bin-VERSION-x86_64.pkg.tar.zst
```

Pacman installs the required runtime libraries. Open **Talis** from your
application menu when installation finishes.

### Portable archive

Extract the archive into a permanent folder, then open a terminal there and run:

```sh
./talis
```

On GNOME Wayland, first run `bash setup-gnome.sh` from that folder, then
`./talis` to finish setup. Keep the extracted folder in place while using Talis.
See the [portable requirements and setup notes](packaging/PORTABLE.md).

## First launch

- Start music in a player that shares playback information with the desktop.
  Talis follows the playing app automatically; choose a specific player under
  **General → Music source** if you prefer.

- Use the turntable's playback and volume controls when the selected player
  makes them available. Cover art appears when supplied by the player.

- Open **Settings** to change appearance and performance options. Some options
  depend on your desktop integration.

- Enable **Run on startup** in General settings if you want Talis to start
  when you sign in, where available.

### GNOME Wayland

The Debian and Arch packages include the desktop companion.
Follow Talis's setup prompt; a fresh installation may need one logout and login.
Once enabled, the companion starts Talis when you sign in. See
[GNOME setup and troubleshooting](integrations/gnome/README.md).

### MATE and Xfce

On MATE, enable **Advanced → MATE compatibility** to show Talis on the desktop.
This mode hides desktop icons while active.

On Xfce, desktop icons are currently covered while Talis runs.

## Desktop compatibility

The following combinations have been tested:

| Distribution | Desktop environments |
| --- | --- |
| Ubuntu | GNOME, MATE |
| Linux Mint | Cinnamon |
| CachyOS | GNOME, KDE Plasma, Hyprland, Xfce |

The `.deb` includes X11 and GNOME Wayland integration. The Arch package and
portable archive also include native Wayland integration for desktops such as
KDE Plasma and Hyprland. GNOME integration currently supports one monitor;
scene and resolution switching are not yet available in that mode.

Talis uses GTK 4.8+ and OpenGL 3.3. Use working graphics drivers for hardware
acceleration; software rendering may reduce performance. Other distributions
and desktop versions may work, but the tested combinations are not a guarantee
for every setup.

## Help and updates

If something goes wrong, open **Advanced → Debug info → Copy debug info** and
include it with a description or screenshot in a
[bug report](https://github.com/AzizStark/talis/issues).

Quit Talis before installing an update. Saved settings are preserved.

## Building from source

Want to build or contribute? See the [developer guide](DEVELOPMENT.md).
Source-build dependencies are separate from the requirements for installed users.

## License

Application code: [GPL-3.0-only](LICENSE). Scene and third-party asset terms:
[licensing and credits](LICENSING.md).
