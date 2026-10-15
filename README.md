# Talis

A music-aware turntable scene for the Linux desktop, with MPRIS playback controls.

## Requirements

- Rust 1.85+ and Cargo ([rustup](https://rustup.rs/)).
- GTK 4.8+, a C compiler, `pkg-config` and OpenGL 3.3.
- `curl` for remote cover art. Blender is not needed to build or run the app.

Install the native build dependencies for your distribution:

```sh
# Debian / Ubuntu / Linux Mint
sudo apt install build-essential git pkg-config libgtk-4-dev curl

# Fedora
sudo dnf install gcc git pkgconf-pkg-config gtk4-devel curl

# Arch / CachyOS
sudo pacman -S --needed base-devel git pkgconf gtk4 curl
```

## Build and run

```sh
git clone https://github.com/AzizStark/talis.git
cd talis
bash scripts/run.sh
```

The source launcher builds the release binary before starting. Quit a running
instance before testing a new build. To build only:

```sh
bash scripts/build.sh
```

Native Wayland (Plasma, Sway, Hyprland) additionally needs **gtk4-layer-shell**
development/runtime packages (`gtk4-layer-shell-devel` on Fedora,
`gtk4-layer-shell` on Arch).
The launcher selects X11 or native Wayland automatically.
**GNOME Wayland** instead uses the [GNOME companion extension](integrations/gnome/README.md).

## Development

Debug launch (X11):

```sh
cargo run --bin talis -- --desktop --arm --arm-flow --now-playing
```

```sh
cargo test --locked
```

Source builds locate `assets/` in the checkout.
See [scene baking](blender/README.md) for editing and regenerating the artwork.
See [release packaging](packaging/README.md) for `.deb`, archive, and GNOME companion builds.

## License

Application code: [GPL-3.0-only](LICENSE). Scene and third-party asset terms:
[licensing and credits](LICENSING.md).
