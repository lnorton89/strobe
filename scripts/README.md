# scripts

| Script | What it does |
|---|---|
| `fetch-angle.ps1` | Downloads ANGLE (from a pinned, hash-checked Electron release) and copies `libEGL.dll`, `libGLESv2.dll` and `d3dcompiler_47.dll` beside the built executables in `target\release` and `target\debug`. Run it once on Windows before trying `--gl angle`. |
