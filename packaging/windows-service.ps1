[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateSet('Install','Remove','Stop')][string]$Action,
    [Parameter(Mandatory=$true)][string]$Binary,
    [string]$DataDirectory
)
$ErrorActionPreference = 'Stop'
$Binary = [IO.Path]::GetFullPath($Binary)
$Service = Get-CimInstance Win32_Service -Filter "Name='ShinyHarbor'"
if ($Service -and !$Service.PathName.StartsWith(('"' + $Binary + '" '), [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Another ShinyHarbor installation owns this service. Manage that installation first.'
}
if ($Service) {
    Stop-Service ShinyHarbor -ErrorAction Stop
    (Get-Service ShinyHarbor).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(75))
}
if ($Action -eq 'Remove') {
    if ($Service) { & sc.exe delete ShinyHarbor; if ($LASTEXITCODE -ne 0) { throw 'Could not remove service.' } }
    exit 0
}
if ($Action -eq 'Stop') { exit 0 }
if (!$DataDirectory) { throw 'DataDirectory is required to install the service.' }
$DataDirectory = [IO.Path]::GetFullPath($DataDirectory)
if ($DataDirectory.TrimEnd('\') -eq [IO.Path]::GetPathRoot($DataDirectory).TrimEnd('\')) {
    throw 'Choose a dedicated data folder, not the root of a drive.'
}
foreach ($Path in @($Binary, $DataDirectory)) {
    $Root = [IO.Path]::GetPathRoot($Path)
    if ($Root.StartsWith('\\') -or ([IO.DriveInfo]::new($Root)).DriveType -eq 'Network') {
        throw 'The LocalService background service requires local disks. Use Start ShinyHarbor for a mapped/network drive, or choose a local installation and data folder.'
    }
}
$Config = Join-Path $DataDirectory 'harbor.toml'
& $Binary check --config $Config
if ($LASTEXITCODE -ne 0) { throw 'Configuration validation failed.' }
$PathsJson = & $Binary paths --config $Config
if ($LASTEXITCODE -ne 0) { throw 'Could not resolve configured service directories.' }
$Paths = $PathsJson | ConvertFrom-Json
$State = [IO.Path]::GetFullPath($Paths.state_dir)
if ($State.StartsWith('\\?\')) { $State = $State.Substring(4) }
if (!$State.StartsWith($DataDirectory.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'For the installed service, state_dir must be a subfolder of the selected data folder.'
}
# Validate before changing any permissions. Never follow a junction into another folder.
for ($Current = $State; $Current; $Current = [IO.Path]::GetDirectoryName($Current)) {
    if ((Test-Path -LiteralPath $Current) -and ((Get-Item -LiteralPath $Current -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "The service data path cannot contain a junction or symbolic link: $Current"
    }
}
# TLS keys may deliberately have inheritance disabled. Grant only this service
# read access to the configured files, never to the local CA or its signing key.
foreach ($TlsPath in $Paths.tls_files) {
    $TlsPath = [IO.Path]::GetFullPath($TlsPath)
    if ($TlsPath.StartsWith('\\?\')) { $TlsPath = $TlsPath.Substring(4) }
    $TlsRoot = [IO.Path]::GetPathRoot($TlsPath)
    if ($TlsRoot.StartsWith('\\') -or ([IO.DriveInfo]::new($TlsRoot)).DriveType -eq 'Network') {
        throw 'The LocalService certificate and key must be on a local disk.'
    }
    for ($Current = $TlsPath; $Current; $Current = [IO.Path]::GetDirectoryName($Current)) {
        if ((Get-Item -LiteralPath $Current -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "The TLS path cannot contain a junction or symbolic link: $Current"
        }
    }
    & icacls $TlsPath /grant '*S-1-5-19:R'
    if ($LASTEXITCODE -ne 0) { throw 'Cannot grant the service read access to its TLS certificate/key.' }
    & icacls ([IO.Path]::GetDirectoryName($TlsPath)) /grant '*S-1-5-19:RX'
    if ($LASTEXITCODE -ne 0) { throw 'Cannot grant the service access to its TLS directory.' }
}
# The installer owns only this data directory; R needs read/execute access.
& icacls $Binary /grant '*S-1-5-19:RX'
if ($LASTEXITCODE -ne 0) { throw 'Cannot grant service access to the executable.' }
& icacls ([IO.Path]::GetDirectoryName($Binary)) /grant '*S-1-5-19:RX'
if ($LASTEXITCODE -ne 0) { throw 'Cannot grant service access to the program directory.' }
& icacls $DataDirectory /grant '*S-1-5-19:(OI)(CI)RX'
if ($LASTEXITCODE -ne 0) { throw 'Cannot grant service read access to application data.' }
# Allow atomic configuration writes, without granting modification of app files.
& icacls $DataDirectory /grant '*S-1-5-19:(W)' '*S-1-3-0:(OI)(CI)(IO)F'
if ($LASTEXITCODE -ne 0) { throw 'Cannot grant service configuration-directory write access.' }
& icacls $Config /grant '*S-1-5-19:M'
if ($LASTEXITCODE -ne 0) { throw 'Cannot grant service configuration write access.' }
New-Item -ItemType Directory -Path $State -Force | Out-Null
& icacls $State /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-19:(OI)(CI)M'
if ($LASTEXITCODE -ne 0) { throw 'Could not protect service state.' }
$CommandLine = '"{0}" service --config "{1}"' -f $Binary,$Config
if ($Service) {
    $Changed = Invoke-CimMethod -InputObject $Service -MethodName Change -Arguments @{
        PathName=$CommandLine; StartMode='Automatic'; StartName='NT AUTHORITY\LocalService'; StartPassword=''
    }
    if ($Changed.ReturnValue -ne 0) { throw "Could not update the service configuration ($($Changed.ReturnValue))." }
} else {
    $Credential = New-Object System.Management.Automation.PSCredential('NT AUTHORITY\LocalService', (New-Object System.Security.SecureString))
    New-Service -Name ShinyHarbor -DisplayName 'ShinyHarbor' -BinaryPathName $CommandLine -StartupType Automatic -Credential $Credential | Out-Null
}
& sc.exe failure ShinyHarbor reset= 86400 actions= restart/5000/restart/15000
if ($LASTEXITCODE -ne 0) { throw 'Could not configure service recovery.' }
Start-Service ShinyHarbor
Write-Host 'ShinyHarbor is running. Configured addresses:'
& $Binary open apps --print --config $Config
& $Binary open dashboard --print --config $Config
& $Binary open admin --print --config $Config
