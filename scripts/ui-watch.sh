#!/usr/bin/env bash
# Put the sandbox display on your screen, so a run of ui-test.sh can be watched as it
# happens.
#
#   nix develop .#testing --command scripts/ui-watch.sh
#
# It leaves a virtual display and a viewer running and returns. Every ui-test.sh run after
# that draws into the same display when it is told to use it:
#
#   GG_DISPLAY=:97 nix develop .#testing --command scripts/ui-test.sh . <<'STEPS'
#   ...
#
# Running it again does nothing if the display and the viewer are already up. `--stop`
# takes them down.
set -euo pipefail

display="${GG_DISPLAY:-:97}"
port=$((5900 + ${display#:}))
width="${GG_WIDTH:-1600}"
height="${GG_HEIGHT:-1000}"

if [ "${1:-}" = "--stop" ]; then
  pkill -f "x11vnc -display $display" 2>/dev/null || true
  pkill -f "vncviewer localhost:$port" 2>/dev/null || true
  pkill -f "Xvfb $display" 2>/dev/null || true
  echo "stopped $display"
  exit 0
fi

for tool in Xvfb x11vnc vncviewer xdotool; do
  command -v "$tool" >/dev/null || {
    echo "$tool missing: run inside 'nix develop .#testing'" >&2
    exit 1
  }
done

if ! DISPLAY="$display" xdotool getdisplaygeometry >/dev/null 2>&1; then
  Xvfb "$display" -screen 0 "${width}x${height}x24" -nolisten tcp >/dev/null 2>&1 &
  disown
  for _ in $(seq 1 40); do
    DISPLAY="$display" xdotool getdisplaygeometry >/dev/null 2>&1 && break
    sleep 0.25
  done
  echo "display $display is up"
fi

if ! pgrep -f "x11vnc -display $display" >/dev/null; then
  # WAYLAND_DISPLAY has to go: x11vnc sees it and refuses to run at all, even when what
  # it is asked to serve is an X display.
  env -u WAYLAND_DISPLAY x11vnc -display "$display" -rfbport "$port" \
    -localhost -nopw -forever -shared -quiet >/dev/null 2>&1 &
  disown
  sleep 0.5
  echo "serving it on localhost:$port"
fi

if ! pgrep -f "vncviewer localhost:$port" >/dev/null; then
  # The viewer belongs on the real session, so it is given the desktop's own display.
  env DISPLAY="${GG_REAL_DISPLAY:-:0}" \
    WAYLAND_DISPLAY="${GG_REAL_WAYLAND:-wayland-0}" \
    vncviewer -Shared -ReconnectOnError=0 "localhost:$port" >/dev/null 2>&1 &
  disown
  echo "viewer opened"
fi

echo "drive it with: GG_DISPLAY=$display scripts/ui-test.sh <repo>"
