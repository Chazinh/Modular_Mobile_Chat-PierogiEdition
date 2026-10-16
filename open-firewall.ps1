# Allows only the ModularChat TCP port from the local subnet, including Wi-Fi marked Public.
# Run once, as Administrator.
param([ValidateRange(1, 65535)][int]$Port = 3000)
$ErrorActionPreference = 'Stop'
$ruleName = "ModularChat-LAN-TCP-$Port"

if (Get-NetFirewallRule -Name $ruleName -ErrorAction SilentlyContinue) {
  Set-NetFirewallRule -Name $ruleName -Enabled True -Direction Inbound -Protocol TCP `
    -LocalPort $Port -Action Allow -Profile Private,Public -RemoteAddress LocalSubnet | Out-Null
} else {
  New-NetFirewallRule -Name $ruleName -DisplayName "ModularChat (LAN TCP $Port)" -Direction Inbound -Protocol TCP `
    -LocalPort $Port -Action Allow -Profile Private,Public -RemoteAddress LocalSubnet | Out-Null
}

Write-Host "Port $Port allowed from the local subnet (Private and Public profiles)."
Get-NetConnectionProfile | Select-Object InterfaceAlias, Name, NetworkCategory | Format-Table
