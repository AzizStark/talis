# Developing Talis

These instructions are for building from source. To install and use a prebuilt
package, start with the [user guide](README.md).

## Build requirements

- Rust 1.85+ and Cargo ([rustup](https://rustup.rs/)).
- GTK 4.8+, a C compiler, `pkg-config` and OpenGL 3.3.
- Git to clone the repository; `curl` for remote cover art.
- Blender is not needed to build or run the application.

Install the native build dependencies for your distribution:

```sh
# Debian / Ubuntu / Linux Mint
sudo apt install build-essential git pkg-config libgtk-4-dev curl

# Fedora
sudo dnf install gcc git pkgconf-pkg-config gtk4-devel curl

# Arch / CachyOS
sudo pacman -S --needed base-devel git pkgconf gtk4 curl
```

Native Wayland builds for Plasma, Sway and Hyprland also need GTK4 layer-shell
development/runtime packages (`gtk4-layer-shell-devel` on Fedora,
`gtk4-layer-shell` on Arch). GNOME Wayland uses the companion instead.

## Build and run

```sh
git clone https://github.com/AzizStark/talis.git
cd talis
bash scripts/run.sh
```

The source launcher selects X11 or native Wayland and builds the release binary
before starting. Quit a running instance before testing a new build. Source
builds locate `assets/` in the checkout.

To build without launching:

```sh
bash scripts/build.sh
```

Use `bash scripts/build.sh wayland` for the native Wayland build or
`bash scripts/build.sh gnome` for the GNOME renderer and client.

Debug launch on X11:

```sh
cargo run --bin talis -- --desktop --arm --arm-flow --now-playing
```

Run the default tests:

```sh
cargo test --locked
```

Some graphical tests are ignored by default and need an isolated display or
session; follow each test's stated requirements instead of running them against
your normal desktop session.

## GNOME companion development

For normal installation, use the [GNOME user guide](integrations/gnome/README.md).
For a source checkout, run the following from the repository root as your
normal user. If another Talis companion is installed, disable it and back it
up outside the extensions directory before replacing it.

```sh
bash scripts/build.sh gnome
talis_extension="$HOME/.local/share/gnome-shell/extensions/talis@azizstark"
mkdir -p "$talis_extension"
install -m644 integrations/gnome/*.js integrations/gnome/*.json "$talis_extension/"
install -m755 target/release/talis-gnome-scene "$talis_extension/"
ln -sfnT "$PWD/assets" "$talis_extension/assets"
```

If you use a custom `CARGO_TARGET_DIR`, adjust the binary path above.
Log out and back in so GNOME discovers the extension, then enable it:

```sh
gnome-extensions enable talis@azizstark
```

The extension starts the renderer; do not also launch `scripts/run.sh`.
Keep the repository in place while using the asset symlink. Disable the
extension before updating its files:

```sh
gnome-extensions disable talis@azizstark
```

The concealed renderer retains a full-size 16:9 camera surface, cropped to
cover the monitor without work-area upscaling. It does not request fullscreen.
Only transport hotspots are placed above desktop windows, below ordinary
application windows, so desktop-icons extensions can remain enabled.

For detailed frame/input/coverage logs, set `"diagnostics": true` in the
installed extension's `scene.json`, then log out/in. Restore `false` when
finished. Keep the field boolean; configurations without it default to quiet.
Errors and visibility transitions are logged normally.

## Artwork and release packages

- [Scene baking](blender/README.md): editing and regenerating the artwork.
- [Release packaging](packaging/README.md): Debian, Arch, portable and GNOME
  companion builds.
