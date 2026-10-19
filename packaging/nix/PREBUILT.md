# Talis for NixOS (prebuilt x86-64 package)

This bundle contains the prebuilt Talis application and scenes. The installer
reuses existing Nix libraries and downloads missing dependencies from
`cache.nixos.org`. An internet connection may be needed; no Rust toolchain,
compilation or extra binary-cache setup is required.

1. Quit the old Talis instance.
2. Extract this archive and open a terminal in its folder.
3. Run as your normal user:

   ```sh
   bash install.sh
   ```

The installer verifies the included checksums, fetches missing signed libraries,
and imports Talis into the Nix store (requesting sudo for that import on a normal
multi-user installation). It then adds Talis to your user profile. Local and
remote builders are disabled: if a prebuilt dependency cannot be downloaded,
installation stops with an error instead of compiling.

The Talis bundle itself is not a signed Nix binary cache; only install a copy
from a source you trust. Dependency signature checks remain enabled.

Open **Talis** from the application menu, or run `talis`.

Turn **Run on startup** off and on once to replace the older startup entry.
On a new installation, simply enable **Run on startup** if wanted.

On KDE Wayland, the scene currently covers Plasma desktop icons and widgets.
This is separate from Talis's application and sidebar icons.

The extracted folder is not needed after a successful profile installation.
After a future package upgrade, re-enable startup to refresh its Nix store path.

To uninstall, use `nix profile list` and then `nix profile remove NAME` with
Talis's exact profile entry name. Saved Talis settings are kept.
