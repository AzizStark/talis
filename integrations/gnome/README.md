# GNOME Wayland

Talis uses a companion extension to display the turntable on the GNOME desktop.
Desktop icons can remain enabled. The integration currently supports one
monitor; scene and resolution switching are not yet available.

## Native packages

The `.deb`, RPM and Arch packages include the companion—no separate download
or terminal setup is needed.

1. Install the Talis package and open **Talis** from your application menu.
2. Follow the setup prompt to enable it on your desktop.
3. If asked, log out and sign back in to finish setup. Save your work first.

Once enabled, the companion starts Talis when you sign in. Opening Talis from
the menu brings up Settings rather than starting another turntable.

## Portable archive

Extract the archive into a permanent folder. Open a terminal in that folder
and run these commands as your normal user:

```sh
bash setup-gnome.sh
./talis
```

Follow the setup prompt, including logout/login if requested. Setup also adds
Talis to your application menu. Keep the extracted folder in place.

## Startup and troubleshooting

- To stop Talis from starting at sign-in, disable its companion in GNOME's
  **Extensions** app. Enable it again to restore the turntable.
- If setup asks for a logout/login, signing out lets GNOME discover the new
  companion files. Try opening Talis again after signing back in.
- If an older manually installed companion conflicts with a packaged one,
  disable it and back it up outside the extensions directory, then log out/in.
  A per-user copy takes precedence over the packaged copy.
- For help, share **Advanced → Debug info → Copy debug info** and any setup
  error in a [bug report](https://github.com/AzizStark/talis/issues).

Setup uses your current GNOME Shell version and reports any actual load error.

## Other installations and development

If your installed runtime does not include the companion, install its matching
release extension ZIP with `gnome-extensions install talis-gnome-VERSION.shell-extension.zip`,
then open Talis to finish setup. Replace the example filename with the actual
downloaded filename; the ZIP still needs the installed Talis runtime and assets.

For source installation, implementation notes and detailed diagnostic logging,
see [GNOME companion development](../../DEVELOPMENT.md#gnome-companion-development).
