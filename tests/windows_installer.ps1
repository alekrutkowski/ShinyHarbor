[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$Installer)
$ErrorActionPreference = 'Stop'
$Installer = (Resolve-Path -LiteralPath $Installer).Path
foreach ($Hive in @('HKCU:','HKLM:')) {
    foreach ($Architecture in @('x64compatible','arm64')) {
        if (Test-Path "$Hive\Software\Microsoft\Windows\CurrentVersion\Uninstall\ShinyHarbor-${Architecture}_is1") {
            throw 'A real ShinyHarbor installation exists; use an isolated runner for this test.'
        }
    }
}
$Root = Join-Path (Resolve-Path (Join-Path $PSScriptRoot '..')).Path ('.build\installer-test-' + [guid]::NewGuid().ToString('N'))
$Program = Join-Path $Root 'program files'
$Data = Join-Path $Root 'data files'
$Config = Join-Path $Data 'harbor.toml'
New-Item -ItemType Directory -Path $Root | Out-Null
$Checks = @()
function Run-Setup {
    $Process = Start-Process -FilePath $Installer -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/CURRENTUSER','/NOICONS','/TASKS=""',('/DIR="' + $Program + '"'),('/DATADIR="' + $Data + '"'),('/LOG="' + (Join-Path $Root 'setup.log') + '"')) -WindowStyle Hidden -Wait -PassThru
    if ($Process.ExitCode -ne 0) { throw "Installer failed ($($Process.ExitCode)); see $Root\setup.log" }
}
try {
    Run-Setup
    if (!(Test-Path -LiteralPath $Config) -or !(Test-Path -LiteralPath (Join-Path $Program 'licenses'))) { throw 'Missing configuration or dependency notices.' }
    & (Join-Path $Program 'shinyharbor.exe') check --config $Config
    if ($LASTEXITCODE -ne 0) { throw 'Installed executable could not validate its configuration.' }
    $Checks += 'silent-install-with-custom-program-and-data-directories'
    $Content = [IO.File]::ReadAllText($Config) + "`n# installer test user edit`n"
    [IO.File]::WriteAllText($Config, $Content, (New-Object Text.UTF8Encoding($false)))
    $Marker = Join-Path $Data 'apps\hello\user-data.txt'
    [IO.File]::WriteAllText($Marker, 'must survive upgrade and uninstall')
    Run-Setup
    if ([IO.File]::ReadAllText($Config) -cne $Content -or !(Test-Path -LiteralPath $Marker)) { throw 'Upgrade overwrote user data.' }
    $Checks += 'upgrade-preserves-configuration-and-application-data'
    Push-Location $env:TEMP
    try {
        $Paths = & (Join-Path $Program 'shinyharbor.exe') paths | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0 -or !$Paths.config.EndsWith('\data files\harbor.toml')) { throw 'Installed data discovery failed.' }
    } finally { Pop-Location }
    $Checks += 'installed-executable-discovers-data-from-unrelated-directory'
} finally {
    $Uninstaller = Join-Path $Program 'unins000.exe'
    if (Test-Path -LiteralPath $Uninstaller) {
        $Process = Start-Process -FilePath $Uninstaller -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART') -WindowStyle Hidden -Wait -PassThru
        if ($Process.ExitCode -ne 0) { throw 'Uninstaller failed.' }
    }
}
if ((Test-Path -LiteralPath (Join-Path $Program 'shinyharbor.exe')) -or !(Test-Path -LiteralPath $Config) -or !(Test-Path -LiteralPath $Marker)) { throw 'Uninstall did not preserve data/remove the executable.' }
$Checks += 'uninstall-removes-program-and-preserves-user-data'
$Report = @{suite='windows-installer'; passed=$true; checks=$Checks} | ConvertTo-Json
New-Item -ItemType Directory -Path (Join-Path $PSScriptRoot '../test-results') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $PSScriptRoot '../test-results/windows-installer.json'), $Report)
Write-Output $Report
