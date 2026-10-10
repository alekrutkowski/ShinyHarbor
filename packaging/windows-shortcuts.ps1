param([Parameter(Mandatory=$true)][string]$Directory, [switch]$Service)
$ErrorActionPreference = 'Stop'
# The service token is private to LocalService and administrators. Ask Windows
# to elevate the terminal monitor; the browser shortcuts remain ordinary links.
$Path = Join-Path $Directory 'Terminal monitor.lnk'
if (!(Test-Path -LiteralPath $Path)) { exit 0 }
$Bytes = [IO.File]::ReadAllBytes($Path)
if ($Bytes.Length -lt 76 -or [BitConverter]::ToUInt32($Bytes,0) -ne 76) { throw 'Invalid terminal shortcut.' }
if ($Service) { $Bytes[21] = $Bytes[21] -bor 0x20 } else { $Bytes[21] = $Bytes[21] -band 0xDF }
[IO.File]::WriteAllBytes($Path, $Bytes)
