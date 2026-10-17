# Release packages

Maintainer build instructions. For installation, see the [main README](../README.md#install),
[portable guide](PORTABLE.md) or [GNOME setup guide](../integrations/gnome/README.md).

Run commands from the repository root unless noted. Source builds need the
[development dependencies](../DEVELOPMENT.md#build-requirements) and a Linux filesystem.

## Portable archive and GNOME ZIP

Build on x86-64 Linux with the release baseline of glibc 2.39 and GTK 4.8.
Also install GTK4 layer-shell development/runtime packages and `zip`.

```sh
bash scripts/release.sh "$PWD/dist/0.1.0" /path/to/gtk4-layer-shell/LICENSE
```

This builds both backends and produces the portable `.tar.gz`, GNOME extension
ZIP and their `SHA256SUMS`. It does **not** build Debian, Arch or RPM packages.
The portable archive includes GTK4 layer-shell; GTK and graphics drivers remain
system dependencies.

Use a new output directory and keep source files unchanged during the build.
The script refuses to replace existing output or publish changed inputs.
`CARGO_TARGET_DIR` can relocate its separate X11 and Wayland build caches.
Cross-compilation is not supported by this release script.

## Debian package

Build from source on Ubuntu 24.04:

```sh
sudo apt install build-essential debhelper pkg-config libgtk-4-dev rustc-1.85 cargo-1.85
dpkg-buildpackage -b -us -uc
```

The `.deb` is written to the parent directory. It includes X11 and the GNOME
companion, but not the native Wayland backend.

## Arch / CachyOS package

On Arch/CachyOS with `base-devel` installed, copy
[`arch/PKGBUILD`](arch/PKGBUILD) and its matching portable release archive into
an empty directory. Run there:

```sh
makepkg -s
```

The `.pkg.tar.zst` is written to that directory. This packages checksum-verified
release binaries, not the current source checkout.

## Fedora RPM

Use the portable release archive matching [`rpm/talis.spec`](rpm/talis.spec):

```sh
sudo dnf install rpm-build desktop-file-utils
mkdir -p dist/rpm/SOURCES
cp /path/to/talis-0.1.0-portable-linux-x86_64.tar.gz dist/rpm/SOURCES/
rpmbuild -bb packaging/rpm/talis.spec --define "_topdir $PWD/dist/rpm"
```

Output is in `dist/rpm/RPMS/x86_64/`. The spec checks the archive before
extracting and retains automatic ELF dependencies.

Both Arch and RPM packages include X11, native Wayland and the GNOME companion.
They use the system layer-shell library rather than the portable private copy.
All native packages leave startup and companion enablement opt-in.

## Before publishing

- Update package versions and revisions, including `debian/changelog`.
- Whenever the portable archive changes, refresh its checksum in both the
  PKGBUILD and RPM spec. Increment `pkgrel` / `Release` for packaging-only changes.
- Include checksums for every published artifact; `release.sh` covers only its
  portable archive and GNOME ZIP.
- Check installation, upgrade, removal and saved settings. Record desktop
  results in the [test matrix](../docs/testing-matrix.md).
