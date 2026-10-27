#Requires -RunAsAdministrator
# Idempotent: configures ONLY the projector Ethernet adapter.
# Static 192.168.50.1/24, no gateway, no DNS, metric above Wi-Fi. Wi-Fi is never modified.
param(
    [string]$EthAlias  = 'Ethernet',
    [string]$WifiAlias = 'Wi-Fi',
    [string]$Ip        = '192.168.50.1',
    [int]$Prefix       = 24,
    [int]$EthMetric    = 500
)
$ErrorActionPreference = 'Stop'

$eth  = Get-NetAdapter -Name $EthAlias
$wifi = Get-NetAdapter -Name $WifiAlias
Write-Host "Wi-Fi    : $($wifi.Name) [$($wifi.InterfaceDescription)] ifIndex=$($wifi.ifIndex) status=$($wifi.Status)"
Write-Host "Ethernet : $($eth.Name) [$($eth.InterfaceDescription)] ifIndex=$($eth.ifIndex) status=$($eth.Status)"

$wifiMetric = (Get-NetIPInterface -InterfaceIndex $wifi.ifIndex -AddressFamily IPv4).InterfaceMetric
if ($EthMetric -le $wifiMetric) { throw "Ethernet metric $EthMetric must exceed Wi-Fi metric $wifiMetric" }

function Invoke-Netsh([string[]]$a) {
    $out = & netsh.exe @a 2>&1
    if ($LASTEXITCODE -ne 0) { throw "netsh $($a -join ' ') failed: $out" }
}
$mask = ([ipaddress]([uint32]::MaxValue -shl (32 - $Prefix) -band [uint32]::MaxValue)).IPAddressToString
if ($Prefix -eq 24) { $mask = '255.255.255.0' }

# 1+2. Static address, no gateway (gateway=none), DHCP off. netsh writes the persistent
#      config even while the cable is unplugged, unlike New-NetIPAddress. Re-running it is a no-op.
Invoke-Netsh @('interface','ipv4','set','address',"name=$EthAlias",'source=static',"address=$Ip","mask=$mask",'gateway=none')

# While the link is down Windows can keep EnableDHCP=1 despite netsh. Force the persistent flag.
$regKey = "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\$($eth.InterfaceGuid)"
if ((Get-ItemProperty $regKey).EnableDHCP -ne 0) {
    Set-ItemProperty $regKey -Name EnableDHCP -Value 0 -Type DWord
    Write-Host "Forced EnableDHCP=0 in registry (takes effect on next link-up)."
}
if ($eth.Status -eq 'Up') {
    Set-NetIPInterface -InterfaceIndex $eth.ifIndex -AddressFamily IPv4 -Dhcp Disabled
}

# Fixed high metric so Wi-Fi always wins.
Set-NetIPInterface -InterfaceIndex $eth.ifIndex -AddressFamily IPv4 -AutomaticMetric Disabled -InterfaceMetric $EthMetric

# 3. No default gateway on Ethernet.
foreach ($store in 'ActiveStore','PersistentStore') {
    Get-NetRoute -InterfaceIndex $eth.ifIndex -DestinationPrefix '0.0.0.0/0' -PolicyStore $store -ErrorAction SilentlyContinue |
        Remove-NetRoute -Confirm:$false -ErrorAction SilentlyContinue
}

# 4. No DNS servers, no DNS registration.
Invoke-Netsh @('interface','ipv4','set','dnsservers',"name=$EthAlias",'source=static','address=none','register=none','validate=no')
Set-DnsClientServerAddress -InterfaceIndex $eth.ifIndex -ResetServerAddresses
# Stale lease data from an old DHCP network would otherwise still be offered as DNS servers.
foreach ($v in 'DhcpNameServer','DhcpIPAddress','DhcpSubnetMask','DhcpServer','DhcpDefaultGateway','DhcpDomain') {
    Remove-ItemProperty $regKey -Name $v -ErrorAction SilentlyContinue
}
Set-ItemProperty $regKey -Name NameServer -Value ''
Set-DnsClient -InterfaceIndex $eth.ifIndex -RegisterThisConnectionsAddress $false

# Report
Write-Host "`n--- Result ---"
Get-NetIPInterface -AddressFamily IPv4 -InterfaceIndex $eth.ifIndex,$wifi.ifIndex |
    Format-Table InterfaceAlias,Dhcp,InterfaceMetric,ConnectionState -AutoSize | Out-String | Write-Host
Get-NetIPAddress -AddressFamily IPv4 -InterfaceIndex $eth.ifIndex | Format-Table InterfaceAlias,IPAddress,PrefixLength -AutoSize | Out-String | Write-Host
Write-Host "Ethernet DNS: '$((Get-DnsClientServerAddress -InterfaceIndex $eth.ifIndex -AddressFamily IPv4).ServerAddresses -join ',')'"
Get-NetRoute -DestinationPrefix '0.0.0.0/0' | Format-Table InterfaceAlias,NextHop,RouteMetric,InterfaceMetric -AutoSize | Out-String | Write-Host
