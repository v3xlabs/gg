#!/usr/bin/env bash
# Drive gg on a private virtual X display and capture what it draws.
#
#   nix develop .#testing --command scripts/ui-test.sh <repo> <<'STEPS'
#   shot opened
#   click 1300 200
#   STEPS
#
# Steps, one per line:
#   shot <name>            capture the screen
#   click <x> <y>          move there and click
#   rightclick <x> <y>     move there and click the right button
#   modclick <key> <x> <y> click with a modifier held down
#   hover <x> <y>          move there and stay
#   drag <x1> <y1> <x2> <y2>   press, move across, release
#   key <keysym>           press one key, chords like ctrl+k included
#   type <text>            type a line
#   resize <w> <h>         resize the window
#   wait <seconds>         do nothing for a while
#
# Run it through `nix develop .#testing`, never `path:.#testing`: a path flake copies the
# whole directory into the nix store, target and all, on every change.
#
# Captures land in .tmp/shots as 001-name.png, 002-name.png and so on, counting on from
# whatever is already there, and .tmp/shots/latest.png always points at the newest. An
# image viewer left open on that file follows a run as it happens.
#
# The display is also served over VNC on localhost, so a viewer pointed at the port the
# script prints watches the run live. GG_WATCH=0 turns that off, and GG_HOLD=<s>
# keeps the window up after the last step so there is something to look at.
set -euo pipefail

repo="${1:?usage: ui-test.sh <repo> [out-dir]}"
outdir="${2:-.tmp/shots}"
binary="${GG_BIN:-./target/debug/gg}"
width="${GG_WIDTH:-1600}"
height="${GG_HEIGHT:-1000}"

for tool in Xvfb xdotool import bwrap; do
  command -v "$tool" >/dev/null || {
    echo "$tool missing: run inside 'nix develop .#testing'" >&2
    exit 1
  }
done

# A display of its own unless ui-watch.sh handed one over, in which case the run draws into
# the display already on screen and leaves it up.
attached=""
display="${GG_DISPLAY:-}"
if [ -n "$display" ]; then
  attached="yes"
else
  for candidate in $(seq 90 96); do
    [ -e "/tmp/.X11-unix/X${candidate}" ] || { display=":${candidate}"; break; }
  done
  [ -n "$display" ] || { echo "no free display between :90 and :96" >&2; exit 1; }
fi

mkdir -p "$outdir"

# A run of gg writes the repositories it opened and the theme it was left on, and that file
# belongs to the copy the reader runs.
export XDG_DATA_HOME="${GG_DATA_HOME:-$PWD/.tmp/sandbox-data}"
mkdir -p "$XDG_DATA_HOME"

xvfb_pid=""
app_pid=""
vnc_pid=""
# Every kill has to be allowed to fail: one already gone must not stop the rest.
cleanup() {
  for pid in "$app_pid" "$vnc_pid" "$xvfb_pid"; do
    if [ -n "$pid" ]; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  wait 2>/dev/null || true
}
trap cleanup EXIT

if [ -z "$attached" ]; then
  Xvfb "$display" -screen 0 "${width}x${height}x24" -nolisten tcp >/dev/null 2>&1 &
  xvfb_pid=$!
fi

# The socket file appears before the server will answer on it, so ask it something.
for _ in $(seq 1 40); do
  DISPLAY="$display" xdotool getdisplaygeometry >/dev/null 2>&1 && break
  sleep 0.25
done
DISPLAY="$display" xdotool getdisplaygeometry >/dev/null 2>&1 || {
  echo "the virtual display never came up" >&2
  exit 1
}

if [ -z "$attached" ] && [ "${GG_WATCH:-1}" != "0" ] && command -v x11vnc >/dev/null; then
  vnc_port=$((5900 + ${display#:}))
  # WAYLAND_DISPLAY has to go here too, or x11vnc refuses to serve an X display at all.
  env -u WAYLAND_DISPLAY x11vnc -display "$display" -rfbport "$vnc_port" \
    -localhost -nopw -forever -shared -quiet >/dev/null 2>&1 &
  vnc_pid=$!
  echo "watching: vncviewer localhost:$vnc_port"
fi

# gg shells out to real git, so it runs sandboxed here for the same reason the tests do:
# nothing it spawns can read the machine's git configuration or reach a signing key. The
# repository under test is mounted at /repo and the build directory at /build, so no path
# from the developer's home exists inside, the X socket and the state directory aside.
#
# WAYLAND_DISPLAY has to go, or winit ignores DISPLAY and talks to the real compositor.
env -u WAYLAND_DISPLAY bwrap \
  --ro-bind /nix/store /nix/store \
  --ro-bind "$PWD/target" /build \
  --bind "$(cd "$(dirname "$repo")" && pwd)" /repo \
  --bind "$XDG_DATA_HOME" /state \
  --ro-bind /tmp/.X11-unix /tmp/.X11-unix \
  --ro-bind-try /etc/fonts /etc/fonts \
  --ro-bind-try /run/opengl-driver /run/opengl-driver \
  --dev-bind-try /dev/dri /dev/dri \
  --proc /proc \
  --dev /dev \
  --tmpfs /tmp/home \
  --setenv HOME /tmp/home \
  --setenv XDG_DATA_HOME /state \
  --setenv DISPLAY "$display" \
  --setenv LD_LIBRARY_PATH "${LD_LIBRARY_PATH:-}" \
  --unsetenv SSH_AUTH_SOCK \
  --unsetenv GPG_AGENT_INFO \
  --unshare-user --unshare-pid --unshare-net --unshare-uts \
  --die-with-parent \
  --chdir /repo \
  -- "/build/debug/$(basename "$binary")" "/repo/$(basename "$repo")" \
  > "$outdir/app.log" 2>&1 &
app_pid=$!

window=""
for _ in $(seq 1 80); do
  if ! kill -0 "$app_pid" 2>/dev/null; then
    echo "gg exited before mapping a window:" >&2
    cat "$outdir/app.log" >&2
    exit 1
  fi
  window=$(DISPLAY="$display" xdotool search --name gg 2>/dev/null | head -1 || true)
  [ -n "$window" ] && break
  sleep 0.25
done
[ -n "$window" ] || { echo "no gg window appeared" >&2; cat "$outdir/app.log" >&2; exit 1; }

# windowactivate needs a window manager to honour _NET_ACTIVE_WINDOW and there is none
# here, so set the input focus directly or key events go nowhere.
DISPLAY="$display" xdotool windowactivate --sync "$window" 2>/dev/null || true
DISPLAY="$display" xdotool windowfocus --sync "$window" 2>/dev/null || true
sleep 1

# Counting on from what is already there, so a run does not overwrite an earlier one.
next=$(find "$outdir" -maxdepth 1 -name '[0-9][0-9][0-9]-*.png' -printf '%f\n' 2>/dev/null |
  sed 's/-.*//' | sort -n | tail -1)
next=$((10#${next:-0} + 1))

while read -r action argument rest; do
  case "$action" in
    ""|"#"*) ;;
    shot)
      sleep 0.4
      file=$(printf '%s/%03d-%s.png' "$outdir" "$next" "${argument:-shot}")
      DISPLAY="$display" import -window root "$file"
      ln -sf "$(basename "$file")" "$outdir/latest.png"
      echo "captured $file"
      next=$((next + 1))
      ;;
    click)
      DISPLAY="$display" xdotool mousemove --sync "$argument" "$rest" click 1
      sleep 0.3
      ;;
    hover)
      DISPLAY="$display" xdotool mousemove --sync "$argument" "$rest"
      sleep 0.3
      ;;
    rightclick)
      DISPLAY="$display" xdotool mousemove --sync "$argument" "$rest" click 3
      sleep 0.3
      ;;
    modclick)
      read -r key x y _ <<<"$argument $rest"
      DISPLAY="$display" xdotool keydown "$key"
      sleep 0.3
      DISPLAY="$display" xdotool mousemove --sync "$x" "$y" click 1
      sleep 0.2
      DISPLAY="$display" xdotool keyup "$key"
      sleep 0.3
      ;;
    drag)
      read -r x1 y1 x2 y2 _ <<<"$argument $rest"
      DISPLAY="$display" xdotool mousemove --sync "$x1" "$y1" mousedown 1
      for step in 1 2 3 4 5 6; do
        DISPLAY="$display" xdotool mousemove --sync \
          $(( x1 + (x2 - x1) * step / 6 )) $(( y1 + (y2 - y1) * step / 6 ))
        sleep 0.05
      done
      DISPLAY="$display" xdotool mouseup 1
      sleep 0.3
      ;;
    key)
      # Refocus each time: the window is not ready to accept focus the instant it maps,
      # and without focus X delivers key events nowhere.
      DISPLAY="$display" xdotool windowfocus --sync "$window" 2>/dev/null || true
      DISPLAY="$display" xdotool key "$argument"
      sleep 0.3
      ;;
    type)
      DISPLAY="$display" xdotool windowfocus --sync "$window" 2>/dev/null || true
      DISPLAY="$display" xdotool type --delay 40 -- "$argument $rest"
      sleep 0.3
      ;;
    resize)
      DISPLAY="$display" xdotool windowsize --sync "$window" "$argument" "$rest"
      sleep 0.5
      ;;
    wait)  sleep "$argument" ;;
    *)     echo "unknown step: $action" >&2; exit 1 ;;
  esac
done

if [ -n "${GG_HOLD:-}" ]; then
  sleep "$GG_HOLD"
fi

echo "done on $display"
