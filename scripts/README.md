# scripts

| Script | What it does |
|---|---|
| `check-linux.sh` | Debian and Ubuntu, WSL2 included: installs the packages, builds the app and the bench, then runs `--diagnose`, the bench headless and on a window, and a control-plus-output screenshot on the default, X11 and Wayland backends, printing PASS, FAIL or SKIP and writing `target/linux-check.txt` (D294). |
| `fetch-angle.ps1` | Downloads ANGLE (from a pinned, hash-checked Electron release) and copies `libEGL.dll`, `libGLESv2.dll` and `d3dcompiler_47.dll` beside the built executables in `target\release` and `target\debug`. Run it once on Windows before trying `--gl angle`. |
