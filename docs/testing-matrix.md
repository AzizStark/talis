# Desktop test matrix

A simple record of manual desktop tests—not a list of systems allowed to run
Talis. An unlisted setup is welcome to try it.

## Results

These are reported manual results; package versions were not recorded for every run.
For each new test, add the package filename in Notes.

| Desktop | Distro | Result | Notes |
| --- | --- | --- | --- |
| GNOME | Ubuntu | Pass | Setup may need logout/login. Scene and resolution controls remain locked. |
| GNOME | CachyOS | Pass | Scene and resolution controls remain locked. |
| GNOME | Fedora | Pass | — |
| KDE Plasma | CachyOS | Pass | Installation and desktop experience passed. |
| KDE Plasma | Fedora | Pass | `talis-0.1.0-1.fc44.x86_64.rpm`; revision 2 tray retest pending. |
| KDE Plasma | NixOS | Issue | Online prebuilt installation/use pass; earlier startup/reboot pass. Plasma desktop icons are covered. |
| KDE Plasma | Kubuntu 26.04 LTS | Pass | `.deb` installation and use passed; session type not recorded. |
| KDE Plasma | KDE neon | Pass | `talis_0.1.0-9_amd64.deb`; installation and corrected tray appearance confirmed. |
| Cinnamon | Linux Mint | Pass | — |
| MATE | Ubuntu | Pass | MATE compatibility enabled; desktop icons hidden. |
| Xfce | CachyOS | Pass | Controls and stacking work; desktop icons covered. |
| Hyprland | CachyOS | Pass | — |
| Sway | — | Not checked | Live desktop test pending; automated headless checks are separate. |

Use **Pass**, **Issue** or **Not checked**. Describe any issue or limitation in
Notes. Keep automated checks separate from manual desktop results.

## Wayland layer regression — 21 October 2026

These are user-reported checks of the issue #3 test builds, which also include
the GL context retry and GTK renderer selection changes.

| Desktop | Distro | Result | Package and scope |
| --- | --- | --- | --- |
| Hyprland (Wayland) | CachyOS | Pass | `talis-bin-0.1.0-5.1-x86_64.pkg.tar.zst`; user confirmed normal operation and placement above the wallpaper with the new Background default. |
| KDE Plasma (Wayland) | NixOS | Pass | `talis-0.1.0-issue3-test-nixos-x86_64.tar.gz`; user confirmed normal operation with the existing Bottom default. Plasma desktop icons remain covered. |

Wallpaper-daemon recreation and login ordering were not explicitly retested.
The passes above cover the tested desktop behavior, not resolution of every
reported issue. The reporter's separate hardware confirmation is recorded below.

Automated headless Sway checks separately verified the layer-shell requests,
scene rendering, and Bottom-layer overlay visibility above the Background
scene, including both explicit layer overrides. These do not replace live
Hyprland or KDE compositor checks.

## OpenGL presentation default — automated checks, 21 October 2026

The expanded X11/Wayland default passed the Wayland-feature library suite
(278 passed, 90 ignored), all-target Clippy, formatting,
and a build without default features. Native and Nix release builds passed.

Isolated X11 smoke tests on GTK 4.14.5 and 4.22.5 rendered day/night scenes
under Muffin and Metacity, preview with Settings, and the GNOME scene client.
Sixteen framebuffer captures contained rendered scene pixels. Logs confirmed
`ngl` on 4.14 and `gl` on 4.22; explicit `gl` and empty `GSK_RENDERER` values
were preserved.

These used Mesa software rendering, not live desktop or NVIDIA tests. Muffin
crashed during forced test-session shutdown, after the scene renders and
window-manager liveness checks had passed. The manual results above were not
rerun for this change; the reporter's separate hardware confirmation follows.

## Reporter confirmation — 21 October 2026

@mickes66 tested an RTX 3080 Ti with NVIDIA 615.78.08 proprietary drivers,
Hyprland and GTK 4.22:

- [Issue #1](https://github.com/AzizStark/talis/issues/1#issuecomment-6321324294):
  without a manual `GDK_DISABLE` override, the initial context creation failed
  and the fresh-process desktop-OpenGL retry rendered the scene successfully.
- [Issue #2](https://github.com/AzizStark/talis/issues/2#issuecomment-6321324683):
  without a manual `GSK_RENDERER` override, Talis selected `gl` and memory stayed
  at about 270 MB for several minutes.
- [Issue #3](https://github.com/AzizStark/talis/issues/3#issuecomment-6321325106):
  the Background default kept Rice Bar overlays visible.

These confirm the compatibility changes on the reported setup, not the root
cause of the original Vulkan/Cairo memory growth. Wallpaper-daemon recreation
and login ordering remain separate, unverified cases.

## Saved Wayland layer setting — automated checks, 21 October 2026

The Advanced → Compatibility → Desktop layer setting passed the Wayland-feature
library suite (284 passed, 91 ignored), the non-desktop library suite (93 passed),
and all-target Clippy with and without the Wayland feature. All desktop binaries
built with the Wayland feature.

Isolated GTK tests verified autosave, reload, reset to Automatic, read-only
environment overrides, unchanged preferences on opening Settings, and debug
information separating the launch layer from the pending preference. X11 window
and existing preferences UI regressions passed; both new UI states were visually
checked.

Headless Sway launches using private saved preferences verified Hyprland and KDE
automatic defaults, explicit saved layers, a relaunch, both environment overrides,
and proper rejection of an invalid override. Protocol traces confirmed the
requested layer and existing namespace; all seven launches rendered a scene and
left the saved preferences unchanged. These are automated software-rendering
checks, not additional live Hyprland/KDE passes.

The NixOS release build and package checks also passed with GTK 4.22.5. Five
additional headless Sway launches verified saved choices and environment
precedence without changing the preferences file.

### Saved layer setting — manual NixOS check

The user tested `talis-0.1.0-wayland-layer-ui-test-nixos-x86_64.tar.gz` on
NixOS/KDE Wayland and confirmed the selector appeared and the layer requests
worked. Automatic requests Bottom on KDE; explicit Background and Bottom request
their respective layers. The `WAYLAND_DEBUG=client` layer-shell trace verifies
the requested layer, not compositor ordering or desktop icon visibility.
This is a focused setting check, not a full rerun of the desktop matrix.

## Quick test

The [0.1.1 release validation](release-validation-0.1.1.md) records the rebuilt
packages, isolated upgrade checks and rendering scope separately from the manual
desktop results above.

- [ ] Install and open Talis from the menu or portable folder.
- [ ] Check the scene, desktop clicks and normal app windows.
- [ ] Play music; check playback buttons, volume and cover art where available.
- [ ] Check Settings, available scene options and sign-in startup where available.
- [ ] Upgrade, remove and reinstall; confirm saved settings survive.

If something fails, add a short note and attach **Advanced → Debug info**.
