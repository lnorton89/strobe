#Requires -RunAsAdministrator
# Idempotent installer for Projector Control (Windows native service: Scheduled Task as SYSTEM).
# Deploys this directory's app\ into C:\ProgramData\ProjectorControl, vendors a private Python
# runtime there, creates the config once, locks down ACLs, opens the API port on the Private
# profile, and (re)starts the service. Safe to re-run after any code or config change.
param(
    [string]$PythonHome = '',   # default: the newest CPython the `py` launcher knows
    [int]$Port = 8780
)
$ErrorActionPreference = 'Stop'
$Src    = $PSScriptRoot
$Root   = 'C:\ProgramData\ProjectorControl'
$App    = Join-Path $Root 'app'
$Py     = Join-Path $Root 'python'
$CfgDir = Join-Path $Root 'config'
$Cfg    = Join-Path $CfgDir 'config.json'
$Logs   = Join-Path $Root 'logs'
$Task   = 'ProjectorControl API'

New-Item -ItemType Directory -Force $Root, $App, $CfgDir, $Logs, $Py | Out-Null

# 1. Code and docs from the checkout. /MIR keeps app\ an exact copy.
robocopy (Join-Path $Src 'app') $App /MIR /NFL /NDL /NJH /NJS /NP /XD __pycache__ | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy app failed ($LASTEXITCODE)" }
Copy-Item (Join-Path $Src 'README.md'), (Join-Path $Src 'setup-network.ps1'), (Join-Path $Src 'install.ps1') $Root -Force

# 2. Private Python runtime, so the SYSTEM service never depends on a user profile or the
#    Store/py-manager aliases (which SYSTEM cannot run).
if (-not $PythonHome) {
    $exe = (& py -3 -c "import sys; print(sys.executable)" 2>$null)
    if ($LASTEXITCODE -ne 0 -or -not $exe) { throw 'No Python 3 found via the py launcher; pass -PythonHome' }
    $PythonHome = Split-Path -Parent $exe.Trim()
}
if ($PythonHome -like '*\WindowsApps*') { throw "Python at $PythonHome is a Store alias; install CPython or pass -PythonHome" }
robocopy $PythonHome $Py /MIR /NFL /NDL /NJH /NJS /NP /XD __pycache__ | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy python failed ($LASTEXITCODE)" }
$PyExe = Join-Path $Py 'python.exe'
& $PyExe -c "import sys; assert sys.version_info >= (3, 9)"
if ($LASTEXITCODE -ne 0) { throw 'vendored python does not run (needs 3.9+)' }

# 3. Config (never overwritten once it exists). Starts from config.example.json with a fresh token.
if (-not (Test-Path $Cfg)) {
    $conf = Get-Content (Join-Path $Src 'config.example.json') -Raw | ConvertFrom-Json
    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $conf.api.token = [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+','-').Replace('/','_')
    $conf.api.port = $Port
    [IO.File]::WriteAllText($Cfg, ($conf | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
    Write-Host "Created $Cfg (new API token generated). Fill in the projector passwords."
} else { Write-Host "Kept existing $Cfg" }

# 4. ACLs. Code: SYSTEM/Admins full, Users read+execute. Config + logs: SYSTEM (service user) + Admins.
function Set-DirAcl([string]$Path, [string[]]$Grants) {
    # Explicit, non-inherited ACL on the directory; everything below it simply inherits.
    & icacls $Path /inheritance:r /grant:r @Grants /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls failed on $Path" }
    if (Get-ChildItem $Path -Force) {
        & icacls (Join-Path $Path '*') /reset /T /C /Q | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "icacls reset failed under $Path" }
    }
}
Set-DirAcl $Root   @('*S-1-5-18:(OI)(CI)F', '*S-1-5-32-544:(OI)(CI)F', '*S-1-5-32-545:(OI)(CI)RX')
Set-DirAcl $CfgDir @('*S-1-5-18:(OI)(CI)F', '*S-1-5-32-544:(OI)(CI)F')
Set-DirAcl $Logs   @('*S-1-5-18:(OI)(CI)F', '*S-1-5-32-544:(OI)(CI)F')

# 5. `pj` command on the machine PATH.
$cmd = "@echo off`r`n`"$PyExe`" `"$App\pjctl.py`" %*`r`nexit /b %ERRORLEVEL%`r`n"
[IO.File]::WriteAllText((Join-Path $Root 'pj.cmd'), $cmd)
$mp = [Environment]::GetEnvironmentVariable('Path', 'Machine')
if (($mp -split ';') -notcontains $Root) {
    [Environment]::SetEnvironmentVariable('Path', ($mp.TrimEnd(';') + ";$Root"), 'Machine')
    Write-Host "Added $Root to machine PATH (open a new shell)"
}

# 6. Firewall. API: inbound TCP on the Private profile (Wi-Fi / Tailscale), this python only.
#    DHCP listen: inbound UDP 67 on the projector Ethernet only (used by `pj discover`, dhcp_reserve).
$ethAlias = (Get-Content $Cfg -Raw | ConvertFrom-Json).network.ethernet_alias
foreach ($name in 'Projector Control API', 'Projector Control DHCP sniff') {
    Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue | Remove-NetFirewallRule
}
New-NetFirewallRule -DisplayName 'Projector Control API' -Direction Inbound -Action Allow -Protocol TCP `
    -LocalPort $Port -Program $PyExe -Profile Private | Out-Null
New-NetFirewallRule -DisplayName 'Projector Control DHCP sniff' -Direction Inbound -Action Allow -Protocol UDP `
    -LocalPort 67 -InterfaceAlias $ethAlias -Program $PyExe | Out-Null

# 7. Service: Scheduled Task at startup as SYSTEM (runs with nobody logged in), restart on failure.
$action    = New-ScheduledTaskAction -Execute $PyExe -Argument "`"$App\server.py`"" -WorkingDirectory $App
$trigger   = New-ScheduledTaskTrigger -AtStartup
$trigger.Delay = 'PT30S'   # let Wi-Fi come up
$principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
$settings  = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable `
    -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $Task -Action $action -Trigger $trigger -Principal $principal -Settings $settings `
    -Description "Projector power HTTP API (Wi-Fi/Tailscale only). See $Root\README.md" -Force | Out-Null

# (Re)start so code/config changes take effect.
Stop-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue
Get-CimInstance Win32_Process -Filter "Name='python.exe'" | Where-Object { $_.CommandLine -like '*ProjectorControl\app\server.py*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
Start-ScheduledTask -TaskName $Task
Write-Host "Task '$Task' registered and started."
