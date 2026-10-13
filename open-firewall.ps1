# Allows the ModularChat port ONLY on Private networks (home Wi-Fi) and only from the local subnet.
# Run once, as Administrator.
param([int]$Port = 3000)

New-NetFirewallRule -DisplayName "ModularChat (LAN)" -Direction Inbound -Protocol TCP `
  -LocalPort $Port -Action Allow -Profile Private -RemoteAddress LocalSubnet | Out-Null

Write-Host "Rule created for port $Port (Private profile, local subnet only)."
Get-NetConnectionProfile | Select-Object InterfaceAlias, Name, NetworkCategory | Format-Table
Write-Host "If the network above is 'Public', switch it to 'Private':"
Write-Host "  Set-NetConnectionProfile -InterfaceAlias 'Ethernet' -NetworkCategory Private"
