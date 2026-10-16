# GNOME Wayland

Companion extension, currently one monitor. GNOME Shell 48 has prior
live-session validation. Fresh-account setup was verified on
Ubuntu 26.04.1 / GNOME 50.1 / Wayland, with desktop icons enabled.
Other Shell versions can attempt startup: setup registers the current version
for Talis only and reports actual load failures. Global extension validation
and other extensions are unchanged.

For a portable bundle, run `bash setup-gnome.sh` inside its permanent extracted
directory as your normal user, then launch `./talis` to finish setup.
Setup registers the Talis menu/dock entry too; no separate patches are needed.

The `.deb` includes the companion. Launch Talis for its consent-based setup
dialog; a new system installation normally needs one logout/login. No terminal
command, ZIP or setup script is needed. On versions absent from the shipped
metadata, setup creates per-user metadata linking to the packaged code, then
requests the usual logout/login. Package updates still update that code.
Manual enabling remains available below after setup.
An older per-user copy takes
precedence over the packaged copy: disable it and back it up outside the
extensions directory before switching, then log out/in.

For other installed runtimes without the companion, install the release extension ZIP with
`gnome-extensions install talis.shell-extension.zip`, then launch Talis to finish
setup. It finds the installed client and assets; no repository symlink is needed.

For source development, run the following from the repository root:

```sh
bash scripts/build.sh gnome
talis_extension="$HOME/.local/share/gnome-shell/extensions/talis@azizstark"
mkdir -p "$talis_extension"
install -m644 integrations/gnome/*.js integrations/gnome/*.json "$talis_extension/"
install -m755 target/release/talis-gnome-scene "$talis_extension/"
ln -sfnT "$PWD/assets" "$talis_extension/assets"
```

Log out and back in so GNOME discovers the extension, then enable it:

```sh
gnome-extensions enable talis@azizstark
```

The extension starts the renderer; do not also launch `scripts/run.sh`.
Login startup is managed by enabling/disabling the companion in Extensions.
Desktop-icons extensions can remain enabled; only the turntable's control
hotspots are placed above desktop windows, below ordinary application windows.
Keep the repository in place while using the asset symlink. Disable the extension
before updating its files:

```sh
gnome-extensions disable talis@azizstark
```

For troubleshooting, set `"diagnostics": true` in the installed extension's
`scene.json`, then log out/in. This enables detailed frame/input/coverage logs;
errors and visibility transitions remain logged normally. Restore `false` when
finished. Keep the field boolean; old configurations without it default to quiet.
