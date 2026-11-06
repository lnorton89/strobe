<!-- @lede -->

Windows has a release you unzip and run. Linux builds from source, on X11 and
Wayland. macOS does not run yet. Strobe is for testing, not yet for production.

<!-- @windows -->

Unzip it anywhere you can write to, for example `C:\Strobe`, and double-click
`strobe.exe`. There is no installer and nothing goes in the registry.

Windows SmartScreen may warn about an unrecognised app the first time: choose
*More info*, then *Run anyway*.

<!-- @windows-files -->

### strobe.exe

The app.

### libEGL.dll, libGLESv2.dll, d3dcompiler_47.dll

ANGLE: OpenGL ES on Direct3D 11, for machines whose OpenGL driver is broken.
They come from an official Electron release, are checked against pinned hashes
when the release is built, and are covered by `LICENSES.chromium.html` beside
them. Keep them next to `strobe.exe`.

### scenes\

The original scenes that ship with Strobe.

### README.md, LICENSE

The project's description and its licence: MIT with the Commons Clause, so
source-available. You may use, change and share Strobe and run it for pay; you
may not sell Strobe itself. A separate `...-symbols.zip` holds `strobe.pdb`, which
you only need to turn a crash report into function names.

<!-- @requirements -->

- A GPU with OpenGL 3.3, or failing that Direct3D 11. Strobe picks one by itself. OpenGL 3.3 is the level of a 2012 Intel HD 4000, though the slowest GPUs tested so far are an Intel UHD 620 and an AMD Vega 3.
- 64-bit Windows. It has been checked on Windows 10 and Windows 11.
- An audio input for audio-reactive scenes. Without one, Strobe plays a synthetic 120 BPM track.

<!-- @linux -->

There is no Linux download yet; you build it. A clean Ubuntu 24.04 needed only
three build packages, and the rest is already on a normal desktop. `rustup`
installs the toolchain `rust-toolchain.toml` pins; distro `rustc` packages are
too old.

<!-- @linux-runtime -->

A minimal server or container also needs the libraries Strobe opens at run time:
EGL and GL, and the X11 or Wayland client libraries. The README lists them, with
Fedora and Arch equivalents.

<!-- @nix -->

`flake.nix` gives a dev shell and a package for x86_64-linux and aarch64-linux.
On x86_64 it built and ran on both X11 and Wayland with Mesa's software driver;
it has not yet been run on NixOS or on a real GPU.

<!-- @source -->

You need the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs
it) and a GPU with OpenGL 3.3, or Direct3D 11 for ANGLE. Windows builds use
the MSVC toolchain, and the ANGLE DLLs come from `scripts/fetch-angle.ps1`.
`strobe --help` lists every flag.

<!-- @after -->

Settings, show-control mappings, logs, crash reports and editor backups live in
the user data directory: `%LOCALAPPDATA%\strobe` on Windows,
`~/.local/share/strobe` on Linux. The user guide covers the first run.
