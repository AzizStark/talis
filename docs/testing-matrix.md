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
| KDE Plasma | NixOS | Pass | — |
| KDE Plasma | Kubuntu 26.04 LTS | Pass | `.deb` installation and use passed; session type not recorded. |
| KDE Plasma | KDE neon | Pass | `talis_0.1.0-9_amd64.deb`; installation and corrected tray appearance confirmed. |
| Cinnamon | Linux Mint | Pass | — |
| MATE | Ubuntu | Pass | MATE compatibility enabled; desktop icons hidden. |
| Xfce | CachyOS | Pass | Controls and stacking work; desktop icons covered. |
| Hyprland | CachyOS | Pass | — |
| Sway | — | Not checked | Live desktop test pending; automated headless checks are separate. |

Use **Pass**, **Issue** or **Not checked**. Describe any issue or limitation in
Notes. Keep automated checks separate from manual desktop results.

## Quick test

- [ ] Install and open Talis from the menu or portable folder.
- [ ] Check the scene, desktop clicks and normal app windows.
- [ ] Play music; check playback buttons, volume and cover art where available.
- [ ] Check Settings, available scene options and sign-in startup where available.
- [ ] Upgrade, remove and reinstall; confirm saved settings survive.

If something fails, add a short note and attach **Advanced → Debug info**.
