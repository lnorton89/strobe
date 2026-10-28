# The projector page's service (D518): what the "ProjectorControl API" task runs, as the user who
# runs Strobe (S4U, no stored password, from boot with nobody logged in). It keeps
# `strobe-projector serve` running and swaps in a build left in next\:
#
#   next\strobe-projector.exe  ->  strobe-projector.exe  (beside this script)
#
# So an update needs no elevation and no access to the running process: leave the new exe in
# next\, and within 5 s this loop stops its own strobe-projector, moves the build over and starts
# it again. (A process started by this S4U task is hidden from the user's own non-elevated shell:
# no path, no command line, so the show launcher could not stop it itself.) The launcher leaves the
# build at logon (README, "Updates"). install.ps1 copies this script here; edit it there, not here.
param([Parameter(Mandatory = $true)][string]$Config)
$ErrorActionPreference = 'Continue'
$Exe  = Join-Path $PSScriptRoot 'strobe-projector.exe'
$Next = Join-Path $PSScriptRoot 'next\strobe-projector.exe'
$Log  = Join-Path (Split-Path -Parent $Config) 'projector-logs\service-updates.log'
# How often a running service looks for a build in next\.
$PollSeconds = 5

function Log([string]$m) {
    try { Add-Content -Path $Log -Value ("[{0:yyyy-MM-dd HH:mm:ss}] {1}" -f (Get-Date), $m) } catch { }
}

New-Item -ItemType Directory -Force (Split-Path -Parent $Log) | Out-Null
if ((Test-Path $Log) -and ((Get-Item $Log).Length -gt 1MB)) {
    Get-Content $Log -Tail 500 | Set-Content "$Log.tmp"; Move-Item "$Log.tmp" $Log -Force
}
Log "=== supervisor started as $env:USERNAME"
while ($true) {
    if (Test-Path $Next) {
        # The exe can stay locked for a moment after its process ends, or while a `pj` command runs.
        $swapped = $false
        for ($i = 0; $i -lt 20 -and -not $swapped; $i++) {
            try { Move-Item $Next $Exe -Force -ErrorAction Stop; $swapped = $true }
            catch { Start-Sleep -Milliseconds 500 }
        }
        if ($swapped) { Log ("swapped in the build from next\ ({0:yyyy-MM-dd HH:mm:ss})" -f (Get-Item $Exe).LastWriteTime) }
        else { Log "could not replace $Exe; running the one there, and trying again at its next start" }
    }
    if (-not (Test-Path $Exe)) { Log "no $Exe; run install.ps1"; Start-Sleep 60; continue }
    # A build that could not be swapped in just now stays in next\; only a new one stops the service.
    $stale = if (Test-Path $Next) { (Get-Item $Next).LastWriteTime } else { $null }
    $start = Get-Date
    $p = Start-Process -FilePath $Exe -ArgumentList @('--config', "`"$Config`"", 'serve') `
        -WorkingDirectory $PSScriptRoot -NoNewWindow -PassThru
    $null = $p.Handle   # keeps the exit code readable after the process ends (Windows PowerShell)
    $update = $false
    while (-not $p.HasExited) {
        Start-Sleep $PollSeconds
        if ((Test-Path $Next) -and ((Get-Item $Next).LastWriteTime -ne $stale)) {
            Log 'a new build is in next\; stopping strobe-projector to swap it in'
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            $null = $p.WaitForExit(10000)
            $update = $true
        }
    }
    if ($update) { continue }
    $up = ((Get-Date) - $start).TotalSeconds
    # A config error ends it at once (exit 2); don't spin on it.
    $wait = if ($up -lt 10) { 30 } else { 3 }
    Log ("strobe-projector exited after {0:N0} s (code {1}); starting it again in {2} s" -f $up, $p.ExitCode, $wait)
    Start-Sleep $wait
}
