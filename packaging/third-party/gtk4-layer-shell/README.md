# GTK4 Layer Shell source

Unmodified GTK4 Layer Shell 1.3.0, licensed under MIT by Sophie Winter.

- Upstream: https://github.com/wmww/gtk4-layer-shell
- Commit: `1c963c51514581c41b9bdae08cdf69171265cdda` (release 1.3.0).
- Archive: https://codeload.github.com/wmww/gtk4-layer-shell/tar.gz/1c963c51514581c41b9bdae08cdf69171265cdda
- Digest: `SHA256SUMS`; license: `LICENSE`; copied-code notices: `NOTICE`.

Debian builds verify and compile this source locally, without downloading
layer-shell during the build. Ubuntu 24.04 does not package the GTK4 version of
layer-shell. Only the compiled library and license notices enter the Talis package;
the library lives privately in `/usr/lib/talis`, not the system library path.
The build also preserves the copyright blocks from the actual protocol XML
files used to generate code in the library.

When updating, review upstream changes, replace the archive and checksum,
update `debian/build.sh`, and rerun package and graphics checks. This is not
the GTK3 `gtk-layer-shell` library.
