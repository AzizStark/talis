# Nix package (maintainer build)

For installation, use the **prebuilt NixOS bundle** and run
`bash install.sh` after extracting it. It reuses installed libraries and downloads
missing signed dependencies from `cache.nixos.org`, with builders disabled.
See [prebuilt installation](PREBUILT.md).
Use the bundle for install-only setup; the root flake builds from source.

This builds Talis from source with Nix-provided libraries and reuses the
checksum-verified release scene data. It includes X11, native Wayland, the GNOME
companion, scenes and icons. Nix supplies the build tools automatically; no
manual Rust installation or FHS environment is needed. The first build can take
a few minutes.

## Build and try locally (maintainers)

From the repository root, with Nix installed:

```sh
nix --extra-experimental-features 'nix-command flakes' build path:.
nix --extra-experimental-features 'nix-command flakes' run path:.
```

To keep it installed in your user profile:

```sh
nix --extra-experimental-features 'nix-command flakes' profile install path:.
```

If you enabled startup with the earlier test bundle, launch this build and turn
**Run on startup** off, then on once. This replaces the old entry with the fixed
backend-specific launcher. On package upgrades, re-enable startup to refresh its
Nix store path.

Use `nix flake check path:.` to check the launchers, packaged assets and desktop
entry. Add the same experimental-features option if flakes are not enabled.

Run `bash packaging/nix/tests/prebuilt.sh` on Linux with Bash, bubblewrap and
standard command-line tools for isolated exporter/installer control-flow checks.
These use mock Nix commands; they do not replace a real store installation test.

This package is not in Nixpkgs.

## Export an install-only bundle

After building and testing `result`, export the prebuilt application and scene data:

```sh
bash packaging/nix/export-prebuilt.sh "$PWD/result" /absolute/new/output-directory
```

The export does not build anything. It bundles store entries that are not signed
by the official NixOS cache, including Talis and its scene data. Signed runtime
dependencies are omitted after checking their availability on `cache.nixos.org`.
The installer fetches only missing dependencies before importing Talis.

The output also contains `SHA256SUMS` for the final archive, usable
with `sha256sum --check SHA256SUMS` from that directory.
No Rust compiler, source checkout or build derivations are needed on
the user's machine. The output directory must not already exist.

The bundle is not a public binary cache. The root flake remains the maintainer's
source-build definition; a public `nix run` command must not be
advertised as download-only without a matching binary cache or binary package.

The package version follows `Cargo.toml`; scene data stays separately pinned to
its published release and checksum. The exporter names the bundle from the
built store output rather than a hard-coded application version.

The Nix wrappers set `TALIS_STARTUP_EXECUTABLE` to the matching backend wrapper.
Without that packaging override, other installations keep using their executable.

## Test status

- Build, dependency resolution, and X11/Wayland launcher checks pass.
- Installation into an isolated Nix profile exposes the launcher and desktop
  menu entry correctly. Profile removal also passes without deleting saved
  settings.
- Headless X11 (Openbox + compositor) and Wayland (Sway) render at 1280×720
  using Nix GTK and Mesa software rendering. Settings text and SVG icons work
  through the wrapped launcher.
- The GNOME renderer also passes a standalone rendering/capture check with Nix
  libraries; this does not verify GNOME Shell integration.
- The earlier native package passed a user-reported NixOS KDE test. Its missing
  icons after sign-in were reproduced and traced to the unwrapped executable.
- The source fix records the validated wrapper instead. Actual app-generated
  entries pass clean-environment restart checks on X11 and on headless Wayland
  with the KDE startup path: SVG icons, backend selection and curl's PATH are
  retained. The user also confirmed installation, sign-in startup and restored
  Talis icons after a reboot in a live NixOS KDE VM.
- Plasma desktop files/folders are separate from Talis's app and sidebar icons:
  the current Wayland scene can cover desktop icons/widgets. Preserving those
  remains a desktop-integration issue, not a Nix packaging fix.
- Hardware-accelerated graphics and GNOME companion enablement remain unverified
  for this native package. The earlier portable/FHS test is separate.
- Final cleanup checks pass: 91 non-GTK library tests (including 13 autostart
  tests), isolated mock-Nix packaging-script checks, formatting and shell syntax.
  These do not replace the earlier real-store and live desktop tests; the full
  Nix build was not repeated after the temporary stores were cleaned up.

The online bundle was also tested in a fresh isolated store: it downloaded the
294 signed runtime dependencies, imported only the two Talis-specific entries,
and installed into a user profile without compiling. A repeat installation
reused the files and kept one active profile entry. All 294 dependency signatures
verified. The application store contents match the earlier VM-tested build.
X11, Wayland and GNOME-renderer command-line checks pass. The user also confirmed
successful installation and use of the online bundle on NixOS KDE. Reboot/startup
was separately confirmed for the identical earlier application build; a reboot
of the online reinstallation was not explicitly reported. The exact multi-user
daemon/import output was not captured.

Mock-Nix checks also cover cache download failures: installation stops before
import/profile changes and does not fall back to a source build.

For the 0.1.1 package rebuild, current desktop/AppStream metadata is installed
from source rather than the older scene-data archive. The package check verifies
the release version. The historical 0.1.0 results above are not a fresh manual
desktop pass.
