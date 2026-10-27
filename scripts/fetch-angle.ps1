# Fetches ANGLE (libEGL.dll, libGLESv2.dll, d3dcompiler_47.dll) for Strobe's
# Direct3D 11 fallback context and places them beside the built executables,
# with the Vulkan loader (vulkan-1.dll) that `--gl angle-vulkan` needs: this
# ANGLE fails to start Vulkan on the loader a GPU driver installs in
# System32, and starts on the one Electron ships with it (D485).
#
# Source: an official Electron release for win32-x64, which ships Google's
# ANGLE build. Both the archive and every extracted DLL are checked against
# pinned SHA-256 hashes, so a changed download fails loudly instead of being
# shipped. To move to a newer ANGLE, bump $Version and all four hashes in
# the same commit (the archive hash is in the release's SHASUMS256.txt).
#
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-angle.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-angle.ps1 -Profile debug
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-angle.ps1 -Destination target\x86_64-pc-windows-msvc\release
#
# A dev build runs this by itself: strobe-gfx's build script calls it with
# -Destination set to the profile's output folder when the DLLs are missing
# there (D498). Run it by hand after bumping $Version, since the build only
# checks that the DLLs are present, not which version they are.
#
# Licences: ANGLE is BSD-3-Clause; d3dcompiler_47.dll is Microsoft's
# redistributable D3D compiler as shipped by Chromium/Electron; vulkan-1.dll
# is the Khronos Vulkan loader (Apache-2.0). Electron's
# LICENSES.chromium.html (extracted alongside) covers them.

param(
    [ValidateSet("release", "debug", "release-fast", "both")]
    [string]$Profile = "both",
    # One folder to fill instead of the profiles under target\; overrides
    # -Profile.
    [string]$Destination = ""
)

$ErrorActionPreference = "Stop"
$Version = "38.2.0"
$ArchiveSha = "4382b317dbbbc0bbf8a301304749324b88207218aac240b670f1c1247c2a02b0"
$Files = @{
    "libEGL.dll"         = "dfd53ccbfce55510f2afdb52703037452370eb5bdf33143829a8b06253015625"
    "libGLESv2.dll"      = "b348ceedd275843cbe344a29dd8550b18e5d433abff695cdf5658470c1e71c67"
    "d3dcompiler_47.dll" = "289767b49dc8525bce262ea06fee4ef9fced2e131a55b25654ce871c1f0498b2"
    "vulkan-1.dll"       = "40e39b8cd62c3292f7c5ad6a2d863f6e3bd3d2c0b1b3a2a0e4d41a61d2a8f758"
}

$Root = Split-Path -Parent $PSScriptRoot
$Cache = Join-Path $Root "target\angle\$Version"
New-Item -ItemType Directory -Force -Path $Cache | Out-Null
$Zip = Join-Path $Cache "electron.zip"
$Url = "https://github.com/electron/electron/releases/download/v$Version/electron-v$Version-win32-x64.zip"

if (-not (Test-Path $Zip) -or (Get-FileHash $Zip -Algorithm SHA256).Hash.ToLower() -ne $ArchiveSha) {
    Write-Host "Downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Zip -UseBasicParsing
}
$Actual = (Get-FileHash $Zip -Algorithm SHA256).Hash.ToLower()
if ($Actual -ne $ArchiveSha) { throw "electron archive hash mismatch: $Actual" }

Add-Type -AssemblyName System.IO.Compression.FileSystem
$Archive = [System.IO.Compression.ZipFile]::OpenRead($Zip)
try {
    foreach ($Name in @($Files.Keys) + "LICENSES.chromium.html") {
        $Entry = $Archive.Entries | Where-Object { $_.FullName -eq $Name }
        if (-not $Entry) { throw "$Name not found in the archive" }
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($Entry, (Join-Path $Cache $Name), $true)
    }
} finally {
    $Archive.Dispose()
}
foreach ($Name in $Files.Keys) {
    $Hash = (Get-FileHash (Join-Path $Cache $Name) -Algorithm SHA256).Hash.ToLower()
    if ($Hash -ne $Files[$Name]) { throw "$Name hash mismatch: $Hash" }
}

# "both" fills every profile people run from: release, debug, and
# release-fast, the one to iterate with (D487).
$Profiles = if ($Profile -eq "both") { @("release", "debug", "release-fast") } else { @($Profile) }
$Dests = if ($Destination) { @($Destination) } else { $Profiles | ForEach-Object { Join-Path $Root "target\$_" } }
foreach ($Dest in $Dests) {
    New-Item -ItemType Directory -Force -Path $Dest | Out-Null
    foreach ($Name in @($Files.Keys) + "LICENSES.chromium.html") {
        Copy-Item (Join-Path $Cache $Name) $Dest -Force
    }
    Write-Host "ANGLE $Version -> $Dest"
}
