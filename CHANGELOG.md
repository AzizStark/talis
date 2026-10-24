# Changelog

## 0.1.1 — 24 October 2026

- Improve Wayland startup compatibility by retrying failed automatic scene
  context creation once with desktop OpenGL on GTK 4.16 and newer.
- Default GTK presentation to OpenGL on X11 and Wayland, avoiding the
  non-OpenGL path associated with reported NVIDIA memory growth. Explicit
  graphics environment overrides remain supported.
- Keep bottom-layer bar decorations visible on Hyprland by placing Talis in
  the Background layer by default.
- Add **Advanced → Compatibility → Desktop layer** for native Wayland desktop
  sessions. Choose Automatic, Background or Bottom; changes apply after
  restarting Talis. Environment overrides take priority.

The original Vulkan/Cairo memory-growth cause remains unconfirmed. A wallpaper
daemon recreating its Background surface can still cover Talis; use Bottom or
restart Talis if needed. Desktop icon integration is unchanged.
