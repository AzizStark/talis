# Talis for NixOS (prebuilt x86-64 package)

This bundle contains Talis and its Nix runtime dependencies.
Installation is offline: no Rust toolchain, compilation, flake evaluation or
binary-cache configuration is needed.

1. Quit the old Talis instance.
2. Extract this archive and open a terminal in its folder.
3. Run as your normal user:

   ```sh
   bash install.sh
   ```

The installer verifies the included checksums, imports the prebuilt files into
the Nix store (requesting sudo on a normal multi-user NixOS installation), then
installs Talis into your user profile with rebuilding disabled. The bundle is
not a signed Nix binary cache; only install a copy from a source you trust.

Open **Talis** from the application menu, or run `talis`.

Turn **Run on startup** off and on once to replace the older startup entry.
On a new installation, simply enable **Run on startup** if wanted.

On KDE Wayland, the scene currently covers Plasma desktop icons and widgets.
This is separate from Talis's application and sidebar icons.

The extracted folder is not needed after a successful profile installation.
After a future package upgrade, re-enable startup to refresh its Nix store path.

To uninstall, use `nix profile list` and then `nix profile remove NAME` with
Talis's exact profile entry name. Saved Talis settings are kept.
