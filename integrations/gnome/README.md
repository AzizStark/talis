# GNOME Wayland

Experimental companion extension for **GNOME Shell 48 and 50**, currently one monitor.
48 has prior live-session validation. Fresh-account setup was verified on
Ubuntu 26.04.1 / GNOME 50.1 / Wayland, with desktop icons enabled.

For a portable bundle, run `bash setup-gnome.sh` inside its permanent extracted
directory as your normal user, log out/in, then enable the companion as below.
Setup registers the Talis menu/dock entry too; no separate patches are needed.

For an installed runtime, install the release extension ZIP with
`gnome-extensions install talis.shell-extension.zip`, then log out/in and enable
it as below. It finds the installed client and assets; no repository symlink is needed.

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
