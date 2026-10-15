#!/usr/bin/env bash
# Strobe on Linux: install the packages, build, and exercise what a Linux
# machine will meet. Written for Debian and Ubuntu, including WSL2 with WSLg.
# Run it from a clone, in the machine's own filesystem (WSL: ~, not /mnt/c).
#
#   scripts/check-linux.sh
#
# It prints PASS, FAIL or SKIP for each step and writes the whole transcript
# to target/linux-check.txt. Send that file back when a step fails.
#
# Switches (environment variables):
#   STROBE_CHECK_NO_APT=1     do not install packages (already done, or no sudo)
#   STROBE_CHECK_NO_BUILD=1   use the binaries already built
#   STROBE_CHECK_BENCH_SIZE   bench resolution, default 640x360 (software GL is slow; a GPU can take 1280x720)
#   STROBE_CHECK_NO_D3D12=1   on WSL2, stay on Mesa's default (llvmpipe) instead of GALLIUM_DRIVER=d3d12
#   GALLIUM_DRIVER            if you set it yourself, the script leaves it alone
#
# It never needs a GPU: Mesa's software renderer is a pass. What it reports
# is which renderer it got, so a D3D12 or hardware renderer is visible.

set -u
cd "$(dirname "$0")/.."
target="${CARGO_TARGET_DIR:-target}"
mkdir -p "$target"
exec > >(tee "$target/linux-check.txt") 2>&1

pass=0 fail=0 skip=0
ok() { echo "PASS  $1"; pass=$((pass + 1)); }
bad() { echo "FAIL  $1"; fail=$((fail + 1)); }
no() { echo "SKIP  $1"; skip=$((skip + 1)); }
step() { echo; echo "=== $1"; }

sudo=""
if [ "$(id -u)" -ne 0 ]; then sudo="sudo"; fi

step "machine"
uname -a
grep -E '^(PRETTY_NAME|VERSION_ID)=' /etc/os-release
wsl=0
if grep -qiE 'microsoft|wsl' /proc/version; then wsl=1; echo "WSL: yes"; else echo "WSL: no"; fi
echo "WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-} DISPLAY=${DISPLAY:-} XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-} PULSE_SERVER=${PULSE_SERVER:-}"
if [ -d /usr/lib/wsl/lib ]; then echo "/usr/lib/wsl/lib:"; ls /usr/lib/wsl/lib; fi
nproc
free -m | head -2

# WSL2 exposes the Windows GPU as /dev/dxg, and Mesa has a d3d12 driver for it,
# but Mesa picks llvmpipe unless told otherwise (an RX 580 ran the bench about
# 3x faster, at twice the pixels, with d3d12). Everything below inherits this.
if [ -e /dev/dxg ] && [ -z "${GALLIUM_DRIVER:-}" ]; then
  if [ "${STROBE_CHECK_NO_D3D12:-0}" = 1 ]; then
    echo "/dev/dxg present; staying on Mesa's default renderer (STROBE_CHECK_NO_D3D12=1)"
  else
    export GALLIUM_DRIVER=d3d12
    echo "/dev/dxg present and GALLIUM_DRIVER unset: using GALLIUM_DRIVER=d3d12 (STROBE_CHECK_NO_D3D12=1 opts out)"
  fi
fi

step "packages"
if [ "${STROBE_CHECK_NO_APT:-0}" = 1 ]; then
  no "apt install (STROBE_CHECK_NO_APT=1)"
else
  # Build: a C compiler, pkg-config, ALSA headers. Run: the libraries winit and
  # glutin open with dlopen. mesa-utils is for glxinfo, alsa-utils and
  # pulseaudio-utils for the audio listing, libasound2-plugins so ALSA's
  # "default" device can reach PulseAudio (what WSLg provides).
  if $sudo apt-get update && $sudo apt-get install -y \
    build-essential pkg-config libasound2-dev git curl ca-certificates \
    libegl1 libgl1 libegl-mesa0 libgl1-mesa-dri mesa-utils \
    libx11-6 libxcursor1 libxi6 libxrandr2 libxkbcommon0 libxkbcommon-x11-0 \
    libwayland-client0 libwayland-cursor0 libwayland-egl1 \
    alsa-utils pulseaudio-utils libasound2-plugins; then
    ok "apt packages"
  else
    bad "apt packages"
  fi
fi

step "rust toolchain"
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
if ! command -v cargo >/dev/null 2>&1; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  . "$HOME/.cargo/env"
fi
# rust-toolchain.toml pins the version; the first cargo call installs it.
cargo --version && rustc --version && ok "rust toolchain" || bad "rust toolchain"

step "build"
if [ "${STROBE_CHECK_NO_BUILD:-0}" = 1 ]; then
  no "cargo build (STROBE_CHECK_NO_BUILD=1)"
else
  start=$SECONDS
  if cargo build --release -p strobe-app -p strobe-bench; then
    ok "cargo build --release -p strobe-app -p strobe-bench ($((SECONDS - start)) s)"
  else
    bad "cargo build"
  fi
fi
strobe="$target/release/strobe"
bench="$target/release/strobe-bench"
if [ ! -x "$strobe" ] || [ ! -x "$bench" ]; then
  echo "no binaries at $target/release, so there is nothing to run"
  bad "binaries present"
  echo; echo "$pass passed, $fail failed, $skip skipped"
  exit 1
fi
ldd "$strobe" | awk '{print "linked: " $1}'

step "strobe --help"
if "$strobe" --help >/dev/null; then ok "strobe --help"; else bad "strobe --help"; fi

step "GL stack"
if command -v glxinfo >/dev/null 2>&1 && [ -n "${DISPLAY:-}" ]; then
  glxinfo -B 2>&1 | grep -E 'name of display|renderer string|OpenGL version|OpenGL core profile version|Device|Video memory' || true
else
  no "glxinfo (no DISPLAY or no mesa-utils)"
fi
"$strobe" --diagnose 2>&1 | sed -n '/^Session/,$p'

# On WSL2 this passed on llvmpipe over EGL (Debian 13, AMD.md A14).
step "headless bench (an EGL device, no window)"
size="${STROBE_CHECK_BENCH_SIZE:-640x360}"
if "$bench" --context headless --size "$size" --frames 20 --warmup 20 --out "$target/bench-linux"; then
  ok "bench headless"
else
  bad "bench headless"
fi

step "bench on a window context"
if [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
  no "bench window (no display)"
elif "$bench" --context window --size "$size" --frames 20 --warmup 20 --out "$target/bench-linux-window"; then
  ok "bench window"
else
  bad "bench window"
fi

# A window run writes the control window and the output window to PNG, which
# proves the window, the GL context, egui and the scene renderer together.
shot() {
  local label="$1"; shift
  rm -f "$target/linux-shot-$label"-*.png
  if "$@" --scene 05 --screenshot "$target/linux-shot-$label" --frames 60 \
    && [ -s "$target/linux-shot-$label-control.png" ] \
    && [ -s "$target/linux-shot-$label-output.png" ]; then
    ok "screenshot $label: $target/linux-shot-$label-{control,output}.png"
  else
    bad "screenshot $label"
  fi
  # What the session log says about the backend, the renderer and audio.
  local log
  log="$(ls -t "$HOME/.local/share/strobe/logs/"strobe-*.log 2>/dev/null | head -1)"
  if [ -n "$log" ]; then
    grep -E 'GL backend|strobe_app::audio|audio' "$log" | head -6 | cut -c1-200
  fi
}

step "app window, default backend"
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
  echo "(winit picks Wayland: WAYLAND_DISPLAY is set)"
elif [ -n "${DISPLAY:-}" ]; then
  echo "(winit picks X11: only DISPLAY is set)"
fi
if [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
  no "app window (no display)"
else
  shot default "$strobe"
fi

step "app window, X11 forced"
if [ -z "${DISPLAY:-}" ]; then
  no "X11 window (DISPLAY is not set)"
else
  shot x11 env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET "$strobe"
fi

step "app window, Wayland forced"
if [ -z "${WAYLAND_DISPLAY:-}" ]; then
  no "Wayland window (WAYLAND_DISPLAY is not set)"
else
  shot wayland env -u DISPLAY "$strobe"
fi

step "audio devices (what Strobe's default input can see)"
arecord -l 2>&1 | head -12 || true
if command -v pactl >/dev/null 2>&1; then pactl list short sources 2>&1 | head -8 || true; fi
echo "Strobe's audio source in the runs above is in the 'strobe_app::audio' lines."
echo "'synthetic' there means no input device was found, which is expected on a"
echo "machine without a microphone; on WSLg it should name the RDP source. If it"
echo "stays synthetic there, ALSA has no 'default' PCM: see the README's WSL2 section"
echo "(~/.asoundrc with 'pcm.!default pulse' and 'ctl.!default pulse')."

step "MIDI"
if [ -e /dev/snd/seq ]; then ok "/dev/snd/seq present"; else no "/dev/snd/seq absent (WSL2 has none; Strobe says MIDI is unavailable and carries on)"; fi

echo
echo "=== summary: $pass passed, $fail failed, $skip skipped (WSL: $wsl)"
echo "transcript: $target/linux-check.txt"
echo "session logs: $HOME/.local/share/strobe/logs"
[ "$fail" -eq 0 ]
