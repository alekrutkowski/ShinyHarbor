[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Archive,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-fA-F]{64}$')][string]$Sha256,
    [string]$Destination = "$env:LOCALAPPDATA\ShinyHarbor"
)
$ErrorActionPreference = 'Stop'
if ((Get-FileHash -Algorithm SHA256 $Archive).Hash -ine $Sha256) { throw 'Checksum mismatch. Nothing was installed.' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$Zip = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path $Archive))
try {
    $Entries = @($Zip.Entries | Where-Object { $_.FullName -match '^shinyharbor-[^/]+/shinyharbor\.exe$' })
    if ($Entries.Count -ne 1) { throw 'Unexpected archive layout.' }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    $Target = Join-Path $Destination 'shinyharbor.exe'
    $Temp = Join-Path $Destination ([System.IO.Path]::GetRandomFileName())
    try {
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($Entries[0], $Temp, $false)
        Move-Item -Force $Temp $Target
    } finally { if (Test-Path $Temp) { Remove-Item $Temp } }
    Write-Host "Installed $Target. Add $Destination to your user PATH or run the full path."
    Write-Host 'Retain the original archive and its dependency notices when redistributing.'
} finally { $Zip.Dispose() }
