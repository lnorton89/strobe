# The projector page's service (D518): what the "ProjectorControl API" task runs, as the user who
# runs Strobe (S4U, no stored password, from boot with nobody logged in). It keeps
# `strobe-projector serve` running, and before each start swaps in a build left in next\:
#
#   next\strobe-projector.exe  ->  strobe-projector.exe  (beside this script)
#
# So an update needs no elevation: copy the new exe into next\ and end the running
# strobe-projector; this loop moves it over and starts it again. The show launcher does that at
# logon (README, "Updates"). install.ps1 copies this script here; edit it there, not here.
param([Parameter(Mandatory = $true)][string]$Config)
$ErrorActionPreference = 'Continue'
$Exe  = Join-Path $PSScriptRoot 'strobe-projector.exe'
$Next = Join-Path $PSScriptRoot 'next\strobe-projector.exe'
$Log  = Join-Path (Split-Path -Parent $Config) 'projector-logs\service-updates.log'

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
        # The exe can stay locked for a moment after its process ends.
        $swapped = $false
        for ($i = 0; $i -lt 20 -and -not $swapped; $i++) {
            try { Move-Item $Next $Exe -Force -ErrorAction Stop; $swapped = $true }
            catch { Start-Sleep -Milliseconds 500 }
        }
        if ($swapped) { Log ("swapped in the build from next\ ({0:yyyy-MM-dd HH:mm:ss})" -f (Get-Item $Exe).LastWriteTime) }
        else { Log "could not replace $Exe; running the one there" }
    }
    if (-not (Test-Path $Exe)) { Log "no $Exe; run install.ps1"; Start-Sleep 60; continue }
    $start = Get-Date
    $p = Start-Process -FilePath $Exe -ArgumentList @('--config', "`"$Config`"", 'serve') `
        -WorkingDirectory $PSScriptRoot -NoNewWindow -PassThru -Wait
    $up = ((Get-Date) - $start).TotalSeconds
    # A config error ends it at once (exit 2); don't spin on it.
    $wait = if ($up -lt 10) { 30 } else { 3 }
    Log ("strobe-projector exited after {0:N0} s (code {1}); starting it again in {2} s" -f $up, $p.ExitCode, $wait)
    Start-Sleep $wait
}
