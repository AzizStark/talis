# Package the checksum-pinned portable release; no compiler or install hooks.
%global debug_package %{nil}
# Preserve the upstream release binaries; retain automatic ELF dependencies.
%global __brp_strip %{nil}
%global __brp_strip_static_archive %{nil}
%global __brp_strip_comment_note %{nil}

Name:           talis
Version:        0.1.0
Release:        2%{?dist}
Summary:        An interactive, animated and music-aware desktop turntable
Vendor:         AzizStark
Packager:       AzizStark <theazizstark@gmail.com>
License:        GPL-3.0-only AND CC-BY-4.0 AND MIT AND LicenseRef-Talis-Artwork
URL:            https://github.com/AzizStark/talis
Source0:        talis-%{version}-portable-linux-x86_64.tar.gz
ExclusiveArch:  x86_64

BuildRequires:  coreutils
BuildRequires:  desktop-file-utils
BuildRequires:  tar
Requires:       bash
Requires:       gtk4 >= 4.8
Requires:       gtk4-layer-shell
Requires:       librsvg2
Requires:       curl
Requires:       ca-certificates
Requires:       glib2
Requires:       libX11
Requires:       libXext
Requires:       libxcb
Requires:       libglvnd-egl
Requires:       libglvnd-glx
Requires:       hicolor-icon-theme

%description
Bring your music to life with an interactive turntable on your desktop.
Watch the record spin and the tonearm move as your music plays, and control
playback right from the turntable. Includes day and night scenes up to 4K,
X11 and native Wayland integration, and the GNOME desktop companion.

%prep
# Check before extracting. A new portable release needs a matching checksum.
echo '43f78510f362795d30629b08a0f4591f1bca9d30c77cffe01225580bb0b2ffa3  %{SOURCE0}' | sha256sum --check --status
%setup -q -n talis-linux-x86_64

%build
# The release archive contains compiled executables.

%install
install -d "%{buildroot}%{_bindir}" "%{buildroot}/usr/lib/talis" \
    "%{buildroot}%{_datadir}/talis" "%{buildroot}%{_datadir}/applications" \
    "%{buildroot}%{_datadir}/icons/hicolor/scalable/apps" \
    "%{buildroot}%{_datadir}/icons/hicolor/scalable/status" "%{buildroot}%{_metainfodir}"
for name in talis-bin talis-wayland-bin talis-gnome-scene launch.sh; do
    install -m755 "usr/lib/talis/$name" "%{buildroot}/usr/lib/talis/$name"
done
install -m644 usr/lib/talis/session.sh "%{buildroot}/usr/lib/talis/session.sh"
ln -s ../lib/talis/launch.sh "%{buildroot}%{_bindir}/talis"
ln -s ../lib/talis/talis-gnome-scene "%{buildroot}%{_bindir}/talis-gnome-scene"

# Keep the shared runtime layout; use the system layer-shell library.
cp -r usr/share/talis/assets "%{buildroot}%{_datadir}/talis/"
install -m644 usr/share/applications/io.github.azizstark.Talis.desktop "%{buildroot}%{_datadir}/applications/"
install -m644 usr/share/icons/hicolor/scalable/apps/io.github.azizstark.Talis.svg "%{buildroot}%{_datadir}/icons/hicolor/scalable/apps/"
install -m644 usr/share/icons/hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg "%{buildroot}%{_datadir}/icons/hicolor/scalable/status/"
install -m644 usr/share/metainfo/io.github.azizstark.Talis.metainfo.xml "%{buildroot}%{_metainfodir}/"
install -d "%{buildroot}%{_datadir}/gnome-shell/extensions/talis@azizstark"
install -m644 usr/share/talis/gnome/*.js usr/share/talis/gnome/*.json \
    usr/share/talis/gnome/LICENSE "%{buildroot}%{_datadir}/gnome-shell/extensions/talis@azizstark/"

%check
desktop-file-validate "%{buildroot}%{_datadir}/applications/io.github.azizstark.Talis.desktop"
test ! -e "%{buildroot}/usr/lib/talis/libgtk4-layer-shell.so.0"
test ! -e "%{buildroot}%{_datadir}/talis/gnome"

%files
%license usr/share/doc/talis/LICENSE usr/share/doc/talis/LICENSING.md
%{_bindir}/talis
%{_bindir}/talis-gnome-scene
/usr/lib/talis/
%{_datadir}/talis/
%{_datadir}/applications/io.github.azizstark.Talis.desktop
%{_datadir}/icons/hicolor/scalable/apps/io.github.azizstark.Talis.svg
%{_datadir}/icons/hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg
%{_metainfodir}/io.github.azizstark.Talis.metainfo.xml
%{_datadir}/gnome-shell/extensions/talis@azizstark/

%changelog
* Sat Oct 17 2026 AzizStark <theazizstark@gmail.com> - 0.1.0-2
- Keep the symbolic tray icon separate from the launcher.
- Include publisher metadata.

* Sat Oct 17 2026 AzizStark <theazizstark@gmail.com> - 0.1.0-1
- Initial RPM with X11, native Wayland and the opt-in GNOME companion.
