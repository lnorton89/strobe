#Requires -RunAsAdministrator
# Idempotent installer for projector control (D502, D518): the `strobe-projector` binary as `pj` on
# the machine PATH and, unless -NoService, the Scheduled Task that serves the web page with Strobe
# closed. The task runs as the user who runs Strobe (S4U: from boot, nobody logged in, no stored
# password), from a folder that user owns, so the show launcher can update it without elevation.
# Replaces the Python install of PR #45 and the SYSTEM task of D502 in place: same task name, the
# config carried over. Safe to re-run after any code or config change.
param(
    # The binary to install. Default: built from this checkout with `cargo build --release`.
    [string]$Exe = '',
    # The config the task and `pj` use. Default: projectors.json in Strobe's data directory for the
    # user running this (the elevated shell's own %LOCALAPPDATA%), the file Settings > Projectors edits.
    [string]$Config = '',
    # Only for a config this creates; an existing config keeps its port.
    [int]$Port = 8780,
    # The account the task runs as. Default: the user running this, who should be the one who runs
    # Strobe (the config and the service folder are in their profile).
    [string]$ServiceUser = "$env:USERDOMAIN\$env:USERNAME",
    # Install `pj` only and remove the task: Strobe serves the page itself while it runs.
    [switch]$NoService,
    # Strobe executables to let through the firewall on the page's port, when Strobe serves the page.
    [string[]]$StrobeExe = @()
)
$ErrorActionPreference = 'Stop'
$Src    = $PSScriptRoot
$Repo   = (Resolve-Path (Join-Path $Src '..\..')).Path
$Root   = 'C:\ProgramData\ProjectorControl'
$OldBin = Join-Path $Root 'bin'
$Task   = 'ProjectorControl API'
if (-not $Config) { $Config = Join-Path $env:LOCALAPPDATA 'strobe\projectors.json' }
$LegacyConfig = Join-Path $Root 'config\config.json'
# The service's own folder, beside the config in the user's profile: the binary, its supervisor
# (pj-service.ps1) and next\, where an update is left for the supervisor to swap in.
$Service   = Join-Path (Split-Path -Parent $Config) 'projector-service'
$Installed = Join-Path $Service 'strobe-projector.exe'

# 1. The binary.
if (-not $Exe) {
    Push-Location $Repo
    try {
        & cargo build --release -p strobe-projector
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    } finally { Pop-Location }
    $Exe = Join-Path $Repo 'target\release\strobe-projector.exe'
}
if (-not (Test-Path $Exe)) { throw "No binary at $Exe" }

# 2. Stop whatever serves now (the old Python server, the SYSTEM task's binary, or a previous
#    supervisor and its binary), so the binary can be replaced and the port is free. The supervisor
#    goes first, or it would start the binary again.
Stop-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue
Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" |
    Where-Object { $_.CommandLine -like '*pj-service.ps1*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
Get-CimInstance Win32_Process -Filter "Name='python.exe' OR Name='strobe-projector.exe'" |
    Where-Object { $_.CommandLine -like '*ProjectorControl*' -or $_.ExecutablePath -eq $Installed } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }

New-Item -ItemType Directory -Force $Root, $Service | Out-Null
Copy-Item $Exe $Installed -Force
Copy-Item (Join-Path $Src 'pj-service.ps1') $Service -Force
Remove-Item (Join-Path $Service 'next') -Recurse -Force -ErrorAction SilentlyContinue
Copy-Item (Join-Path $Src 'README.md'), (Join-Path $Src 'setup-network.ps1'), (Join-Path $Src 'install.ps1') $Root -Force
# The SYSTEM task's copy (D502) is retired with it.
if (Test-Path $OldBin) { Remove-Item $OldBin -Recurse -Force; Write-Host "Removed the SYSTEM install's $OldBin" }

# 3. Config, never overwritten once it exists. The Python install's config moves over as it is (same
#    shape: token, passwords and the Sony's MAC kept); otherwise config.example.json with a new token.
New-Item -ItemType Directory -Force (Split-Path -Parent $Config) | Out-Null
if (-not (Test-Path $Config)) {
    if (Test-Path $LegacyConfig) {
        Copy-Item $LegacyConfig $Config
        Write-Host "Moved the Python install's config to $Config (token and passwords kept)."
    } else {
        $conf = Get-Content (Join-Path $Src 'config.example.json') -Raw | ConvertFrom-Json
        $bytes = New-Object byte[] 32
        [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
        $conf.api.token = -join ($bytes | ForEach-Object { $_.ToString('x2') })
        $conf.api.port = $Port
        [IO.File]::WriteAllText($Config, ($conf | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
        Write-Host "Created $Config (new API token). Fill in the projector passwords in Strobe's Settings > Projectors."
    }
} else { Write-Host "Kept existing $Config" }

# 4. The Python install, retired: its code and private runtime go. Its config (admin-only) and logs stay
#    under $Root\config and $Root\logs; delete them once the new install has been checked.
foreach ($old in 'app', 'python') {
    $path = Join-Path $Root $old
    if (Test-Path $path) { Remove-Item $path -Recurse -Force; Write-Host "Removed the Python install's $old\" }
}

# 5. ACLs. $Root (pj.cmd and the scripts): SYSTEM/Admins full, Users read+execute. The service folder:
#    the service user full, so the show launcher can leave an update in next\ and the supervisor can
#    swap it in; an elevated shell created it, so it is granted explicitly.
& icacls $Root /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX' /Q | Out-Null
if ($LASTEXITCODE -ne 0) { throw "icacls failed on $Root" }
& icacls $Service /grant "${ServiceUser}:(OI)(CI)F" /Q | Out-Null
if ($LASTEXITCODE -ne 0) { throw "icacls failed on $Service" }

# 6. `pj` on the machine PATH, pointed at this config.
$cmd = "@echo off`r`n`"$Installed`" --config `"$Config`" %*`r`nexit /b %ERRORLEVEL%`r`n"
[IO.File]::WriteAllText((Join-Path $Root 'pj.cmd'), $cmd)
$mp = [Environment]::GetEnvironmentVariable('Path', 'Machine')
if (($mp -split ';') -notcontains $Root) {
    [Environment]::SetEnvironmentVariable('Path', ($mp.TrimEnd(';') + ";$Root"), 'Machine')
    Write-Host "Added $Root to machine PATH (open a new shell)"
}

# 7. Firewall. The page: inbound TCP on the Private profile (Wi-Fi / Tailscale), these programs only.
#    DHCP listen: inbound UDP 67 on the projector Ethernet only (`pj discover`, `pj dhcp-reserve`).
#    An update keeps the binary's path, so the rules hold across updates.
$conf = Get-Content $Config -Raw | ConvertFrom-Json
$apiPort = if ($conf.api -and $conf.api.port) { [int]$conf.api.port } else { 8780 }
$ethAlias = if ($conf.network -and $conf.network.ethernet_alias) { $conf.network.ethernet_alias } else { 'Ethernet' }
Get-NetFirewallRule -DisplayName 'Projector Control*' -ErrorAction SilentlyContinue | Remove-NetFirewallRule
New-NetFirewallRule -DisplayName 'Projector Control API' -Direction Inbound -Action Allow -Protocol TCP `
    -LocalPort $apiPort -Program $Installed -Profile Private | Out-Null
New-NetFirewallRule -DisplayName 'Projector Control DHCP sniff' -Direction Inbound -Action Allow -Protocol UDP `
    -LocalPort 67 -InterfaceAlias $ethAlias -Program $Installed | Out-Null
$i = 0
foreach ($strobe in $StrobeExe) {
    $i++
    New-NetFirewallRule -DisplayName "Projector Control API (Strobe $i)" -Direction Inbound -Action Allow -Protocol TCP `
        -LocalPort $apiPort -Program (Resolve-Path $strobe).Path -Profile Private | Out-Null
}

# 8. Service: a Scheduled Task at startup as $ServiceUser with S4U (runs with nobody logged in, no
#    password stored, local access only, which is all it needs), not elevated. It runs the supervisor,
#    which keeps the binary running and swaps in updates. Same name as the earlier installs', so
#    registering replaces them.
if ($NoService) {
    if (Get-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue) {
        Unregister-ScheduledTask -TaskName $Task -Confirm:$false
        Write-Host "Removed task '$Task': Strobe serves the page (Settings > Projectors > Web page)."
    }
    return
}
$supervisor = Join-Path $Service 'pj-service.ps1'
$action    = New-ScheduledTaskAction -Execute 'powershell.exe' -WorkingDirectory $Service `
    -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$supervisor`" -Config `"$Config`""
$trigger   = New-ScheduledTaskTrigger -AtStartup
$trigger.Delay = 'PT30S'   # let Wi-Fi come up
$principal = New-ScheduledTaskPrincipal -UserId $ServiceUser -LogonType S4U -RunLevel Limited
$settings  = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable `
    -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $Task -Action $action -Trigger $trigger -Principal $principal -Settings $settings `
    -Description "Projector power web page (Wi-Fi/Tailscale only), as $ServiceUser. See $Root\README.md" -Force | Out-Null
Start-ScheduledTask -TaskName $Task
Write-Host "Task '$Task' registered and started as ${ServiceUser}: $supervisor keeps strobe-projector serve --config $Config running"
