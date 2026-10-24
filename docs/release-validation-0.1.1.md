# Talis 0.1.1 release validation

Recorded 24 October 2026. These are release-package checks, not a fresh manual
pass on every desktop in the [test matrix](testing-matrix.md).

## Builds and metadata

- Rebuilt portable, Debian, Arch/CachyOS, Fedora RPM, GNOME companion ZIP and
  NixOS downloads from merged main plus the 0.1.1 release metadata.
- Portable/native binaries use the Ubuntu 24.04 glibc 2.39 baseline and GTK
  4.14.5. ELF symbol requirements do not exceed glibc 2.39.
- Debian's source build passed both X11 and native Wayland release test suites.
  The portable bundle includes the private layer-shell library and its combined
  license/protocol notices; Arch and RPM use the system library.
- Arch and RPM recipes verify the portable archive checksum before packaging.
- Nix built against GTK 4.22.5 and passed its package check, including current
  version metadata. Scene data remains independently pinned to 0.1.0.
- The NixOS export contains two Talis-specific store entries; 294 signed
  dependencies are reused or downloaded from the official cache.

## Automated checks

- Wayland-feature library suite: 284 passed, 91 ignored. Main/scene-client and
  CLI integration tests also passed; one graphical CLI test remained ignored.
- Non-desktop library suite: 93 passed.
- All-target Wayland Clippy, formatting, shell syntax, ten Node packaging/GNOME
  test files, mock-Nix packaging tests and offline AppStream validation passed.
- Isolated dpkg, pacman and RPM install/upgrade/removal/reinstall checks preserved
  private test preferences. Debian and RPM upgrades used the published 0.1.0
  packages; Arch used the earlier 0.1.0-5.1 test package.
- Those private native-package databases bypass dependency resolution and
  install hooks; package dependency metadata was inspected separately. They do
  not replace live APT, DNF or pacman dependency-resolution checks.
- The Nix export imported successfully into the cached isolated store. A private
  user profile correctly rejected an older conflicting entry, installed after
  that entry was removed, handled repeat installation, and preserved preferences
  after profile removal. Fresh dependency downloads were not repeated.

## Rendering scope

Software-rendered X11 checks on GTK 4.14.5 and 4.22.5 covered Muffin/Metacity
day/night placement, preview with Settings, explicit GTK renderer overrides and
the standalone GNOME scene client. Sixteen captures contained rendered pixels.
Muffin crashed during forced session shutdown, after rendering and window-manager
liveness checks had passed.

Twelve headless Sway launches across the portable and Nix packages rendered saved
Automatic/Background/Bottom choices, confirmed layer-shell requests and namespace
preservation, honored both environment overrides without rewriting preferences,
and rejected invalid overrides. Captures contained rendered scene pixels.

These are not live Hyprland/KWin, GNOME Shell, hardware-acceleration, sign-in or
reboot tests of the new packages. The earlier reporter's NVIDIA confirmations
remain recorded separately in the test matrix. Desktop icon integration and
wallpaper-daemon recreation limitations are unchanged.
