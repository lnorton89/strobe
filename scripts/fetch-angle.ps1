# Fetches ANGLE (libEGL.dll, libGLESv2.dll, d3dcompiler_47.dll) for Strobe's
# Direct3D 11 fallback context and places them beside the built executables,
# with the Vulkan loader (vulkan-1.dll) that `--gl angle-vulkan` needs: this
# ANGLE fails to start Vulkan on the loader a GPU driver installs in
# System32, and starts on the one Electron ships with it (D485).
#
# Source: an official Electron release for win32-x64, which ships Google's
# ANGLE build. Both the archive and every extracted DLL are checked against
# pinned SHA-256 hashes, so a changed download fails loudly instead of being
# shipped. The pins live in scripts\native-deps.json (D536), the one list
# the build, the release's SBOM and `strobe --diagnose` all read. To move to
# a newer ANGLE, change the version, source and all five hashes there in one
# commit (the archive hash is in the release's SHASUMS256.txt).
#
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-angle.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-angle.ps1 -Profile debug
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-angle.ps1 -Destination target\x86_64-pc-windows-msvc\release
#
# A dev build runs this by itself: strobe-gfx's build script calls it with
# -Destination set to the profile's output folder when the DLLs are missing
# there, or when the `angle.stamp` this writes beside them names another
# version than the manifest (D498, D536), so a bumped pin refreshes every
# profile's DLLs on its next build.
#
# Licences: ANGLE is BSD-3-Clause; d3dcompiler_47.dll is Microsoft's
# redistributable D3D compiler as shipped by Chromium/Electron; vulkan-1.dll
# is the Khronos Vulkan loader (Apache-2.0). Electron's
# LICENSES.chromium.html (extracted alongside) covers them.

param(
    # Called -Profile on the command line, as it always was; inside, a
    # variable named $Profile would shadow PowerShell's own $PROFILE (D538).
    [Alias("Profile")]
    [ValidateSet("release", "debug", "release-fast", "both")]
    [string]$BuildProfile = "both",
    # One folder to fill instead of the profiles under target\; overrides
    # -Profile.
    [string]$Destination = ""
)

$ErrorActionPreference = "Stop"
$Angle = (Get-Content (Join-Path $PSScriptRoot "native-deps.json") -Raw | ConvertFrom-Json).angle
$Version = $Angle.version
$ArchiveSha = $Angle.archive_sha256
$Url = $Angle.source
$Files = @{}
foreach ($File in $Angle.files) { $Files[$File.name] = $File.sha256 }
if (-not $Version -or -not $ArchiveSha -or -not $Url -or $Files.Count -eq 0) {
    throw 'scripts\native-deps.json has no complete "angle" entry'
}

$Root = Split-Path -Parent $PSScriptRoot
$Cache = Join-Path $Root "target\angle\$Version"
New-Item -ItemType Directory -Force -Path $Cache | Out-Null
$Zip = Join-Path $Cache "electron.zip"

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
$Profiles = if ($BuildProfile -eq "both") { @("release", "debug", "release-fast") } else { @($BuildProfile) }
$Dests = if ($Destination) { @($Destination) } else { $Profiles | ForEach-Object { Join-Path $Root "target\$_" } }
foreach ($Dest in $Dests) {
    New-Item -ItemType Directory -Force -Path $Dest | Out-Null
    foreach ($Name in @($Files.Keys) + "LICENSES.chromium.html") {
        Copy-Item (Join-Path $Cache $Name) $Dest -Force
    }
    # Which version these DLLs are, for strobe-gfx's build script: a name
    # that exists says nothing about its contents (D536).
    Set-Content -Path (Join-Path $Dest "angle.stamp") -Value $Version -NoNewline -Encoding ascii
    Write-Host "ANGLE $Version -> $Dest"
}
