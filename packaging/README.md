# Release packages

The native package uses an **Ubuntu 24.04 / Linux Mint 22** build baseline.
It supports X11 and includes the GNOME Wayland companion for Shell 48/50.
The portable bundle also supports the separately tested Wayland integrations;
this does not imply that native distro packages exist for each desktop.

## Debian package

Build on a Linux/POSIX filesystem, not NTFS. On Ubuntu 24.04:

```sh
sudo apt install build-essential debhelper pkg-config libgtk-4-dev rustc-1.85 cargo-1.85
dpkg-buildpackage -b -us -uc
```

The `.deb` is written to the parent directory. Install it with `sudo apt install ./talis_<version>_<arch>.deb`.
Installed launches need no Rust, Cargo, or Blender. Startup remains opt-in.
Quit Talis before upgrading; saved settings are preserved.

On GNOME Wayland, log out/in after installation, then enable the included companion:

```sh
gnome-extensions enable talis@azizstark
```

No separate ZIP or setup script is needed. Enabling remains per-user and opt-in;
the package does not install GNOME on other desktops. A previous per-user copy
of the same extension takes precedence; back it up outside the extensions directory
before switching to the packaged copy, then log out/in.

## Manual archive

From the repository root, with the normal source-build dependencies installed:

```sh
bash scripts/build.sh gnome
bash scripts/stage-release.sh "$PWD/dist/runtime" "$PWD/target/release"
tar -C dist/runtime -czf dist/talis-linux-x86_64.tar.gz usr
```

Extract into a permanent directory and run `./usr/bin/talis`. This archive still
requires compatible system libraries; it is not an AppImage. Use a new/empty
staging directory for each build.

## Portable X11 + Wayland bundle

This adds the native Wayland backend and its GTK4 layer-shell library; GTK and
graphics drivers still come from the system. Build on the oldest supported
system (current test baseline: x86-64, glibc 2.39 / GTK 4.8+).

With GTK4 layer-shell development/runtime packages and `zip` installed, run:

```sh
bash scripts/release.sh "$PWD/dist/0.1.0" /path/to/gtk4-layer-shell/LICENSE
```

This builds X11/GNOME and Wayland into separate Cargo target directories, uses
the layer-shell runtime selected by `pkg-config`, and produces:

- A complete, versioned portable archive, including the GNOME companion.
- A GNOME extension ZIP for runtimes without a packaged companion.
- `SHA256SUMS` for both artifacts.

The output directory must not exist. Failed builds or changed release inputs
do not publish artifacts; existing releases are never replaced. Uncommitted
source is supported, but keep it unchanged during the build. This is consistent
release assembly, not a claim of bit-for-bit reproducible compiler output.
`CARGO_TARGET_DIR` may relocate the two build caches. Cross-compilation is not
supported by this x86-64 release command. `.deb` builds remain separate above.

Extract and run `./talis` inside the bundle. GNOME Shell 48/50 instead needs
`bash setup-gnome.sh` once, log out/in, then extension enabling; the bundle's
README has details. No separate patches or backend selection are needed.
Nothing installs or enables itself just by launching the app.

## GNOME companion

The `.deb` already includes the companion. For other installed runtimes, create
the JavaScript-only extension ZIP separately:

```sh
mkdir -p dist
zip -j dist/talis.shell-extension.zip integrations/gnome/*.js integrations/gnome/*.json LICENSE
```

It requires the installed `talis-gnome-scene` client and scene assets. Current
targets are GNOME Shell 48 and 50, one monitor. Fresh-account installation was
verified on Ubuntu 26.04.1 / GNOME 50.1 / Wayland. Installation does not enable it;
see the [GNOME instructions](../integrations/gnome/README.md).
