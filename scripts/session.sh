# Shared session selection for source and installed launchers. No side effects.
talis_session_backend() {
  local backend=${TALIS_BACKEND:-auto}
  if [[ $backend == auto ]]; then
    if [[ ${XDG_SESSION_TYPE:-} == wayland || (-n ${WAYLAND_DISPLAY:-} && ${XDG_SESSION_TYPE:-} != x11) ]]; then
      backend=wayland
    else
      backend=x11
    fi
  fi
  case "$backend" in
    x11|wayland) printf '%s\n' "$backend" ;;
    *) echo 'TALIS_BACKEND must be auto, x11 or wayland.' >&2; return 2 ;;
  esac
}

talis_session_is_gnome() {
  local desktops=${XDG_CURRENT_DESKTOP:-}
  [[ :${desktops,,}: == *:gnome:* ]]
}
