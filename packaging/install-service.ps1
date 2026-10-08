# Run in an elevated PowerShell. Uses the native Service Control Manager entry point.
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$Config,
    [Parameter(Mandatory=$true)][string]$StateDirectory
)
$ErrorActionPreference = 'Stop'
$Binary = (Resolve-Path $Binary).Path
$Config = (Resolve-Path $Config).Path
if (Get-Service -Name ShinyHarbor -ErrorAction SilentlyContinue) { throw 'ShinyHarbor service already exists. Stop and update it explicitly.' }
& $Binary check --config $Config
if ($LASTEXITCODE -ne 0) { throw 'Configuration validation failed.' }
New-Item -ItemType Directory -Force -Path $StateDirectory | Out-Null
# Inheritance is removed; only SYSTEM, Administrators and LocalService may access state.
# StateDirectory must match state_dir in the TOML file. Review parent-directory ACLs too.
& icacls $StateDirectory /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-19:(OI)(CI)M'
if ($LASTEXITCODE -ne 0) { throw 'Could not protect the state directory.' }
$Command = '"{0}" service --config "{1}"' -f $Binary, $Config
$Credential = New-Object System.Management.Automation.PSCredential('NT AUTHORITY\LocalService', (New-Object System.Security.SecureString))
New-Service -Name ShinyHarbor -DisplayName 'ShinyHarbor' -BinaryPathName $Command -StartupType Automatic -Credential $Credential | Out-Null
Write-Host 'Service installed, not started. Give LocalService read/execute access to R, the config and apps, then run Start-Service ShinyHarbor.'
Write-Host 'Configure an absolute Rscript.exe path. No firewall rules or privileged app accounts were added.'
