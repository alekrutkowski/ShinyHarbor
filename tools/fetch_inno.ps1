[CmdletBinding()]
param([string]$Destination = (Join-Path $PSScriptRoot '../.build/inno'))
$ErrorActionPreference = 'Stop'
$Destination = [IO.Path]::GetFullPath($Destination)
$Download = Join-Path ([IO.Path]::GetDirectoryName($Destination)) 'innosetup-6.7.3.exe'
if (!(Test-Path -LiteralPath (Join-Path $Destination 'ISCC.exe'))) {
    Invoke-WebRequest -Uri 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $Download
    if ((Get-FileHash -LiteralPath $Download -Algorithm SHA256).Hash -ine '9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732') { throw 'Inno Setup compiler checksum mismatch.' }
    $Process = Start-Process -FilePath $Download -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/CURRENTUSER','/NOICONS',('/DIR="' + $Destination + '"')) -WindowStyle Hidden -Wait -PassThru
    if ($Process.ExitCode -ne 0) { throw "Compiler installation failed: $($Process.ExitCode)" }
}
Write-Output (Join-Path $Destination 'ISCC.exe')
