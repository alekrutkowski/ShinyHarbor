; Built by tools/installers.py from a checksum-verified portable release.
#ifndef Version
  #error Version is required
#endif
[Setup]
AppId=ShinyHarbor-{#Architecture}
AppName=ShinyHarbor
AppVersion={#Version}
AppPublisher=Alek Rutkowski
AppPublisherURL=https://github.com/alekrutkowski/ShinyHarbor
DefaultDirName={autopf}\ShinyHarbor
DefaultGroupName=ShinyHarbor
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#Architecture}
ArchitecturesInstallIn64BitMode={#Architecture}
OutputDir={#Output}
OutputBaseFilename=shinyharbor-{#Version}-{#Target}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
LicenseFile={#Payload}\LICENSE
UninstallDisplayIcon={app}\shinyharbor.exe
CloseApplications=yes
RestartApplications=no
AllowNetworkDrive=yes

[Files]
Source: "{#Payload}\shinyharbor.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\VALIDATION.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\SECURITY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\ATTRIBUTIONS.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\THIRD_PARTY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\sbom.cdx.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Payload}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Payload}\packaging\*"; DestDir: "{app}\packaging"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Payload}\validation\*"; DestDir: "{app}\validation"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Payload}\conformance\*"; DestDir: "{app}\conformance"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Payload}\examples\*"; DestDir: "{app}\examples"; Flags: ignoreversion recursesubdirs createallsubdirs

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked
Name: shiny; Description: "Install or update the Shiny R package (requires R and Internet access)"; Flags: unchecked
Name: service; Description: "Run in the background as a Windows service"; Flags: unchecked; Check: IsAdminInstallMode

[Icons]
Name: "{group}\Start ShinyHarbor"; Filename: "{app}\shinyharbor.exe"; Parameters: "--config ""{code:DataDirectory}\harbor.toml"""; WorkingDir: "{code:DataDirectory}"
Name: "{autodesktop}\ShinyHarbor"; Filename: "{app}\shinyharbor.exe"; Parameters: "--config ""{code:DataDirectory}\harbor.toml"""; WorkingDir: "{code:DataDirectory}"; Tasks: desktopicon
Name: "{group}\Open applications"; Filename: "{app}\shinyharbor.exe"; Parameters: "open apps --config ""{code:DataDirectory}\harbor.toml"""; WorkingDir: "{code:DataDirectory}"
Name: "{group}\Usage dashboard"; Filename: "{app}\shinyharbor.exe"; Parameters: "open dashboard --config ""{code:DataDirectory}\harbor.toml"""; WorkingDir: "{code:DataDirectory}"
Name: "{group}\Server administration"; Filename: "{app}\shinyharbor.exe"; Parameters: "open admin --config ""{code:DataDirectory}\harbor.toml"""; WorkingDir: "{code:DataDirectory}"
Name: "{group}\Terminal monitor"; Filename: "{app}\shinyharbor.exe"; Parameters: "tui --config ""{code:DataDirectory}\harbor.toml"""; WorkingDir: "{code:DataDirectory}"
Name: "{group}\Edit configuration"; Filename: "{sys}\notepad.exe"; Parameters: """{code:DataDirectory}\harbor.toml"""
Name: "{group}\Application folders"; Filename: "{code:DataDirectory}\apps"
Name: "{group}\Installation guide"; Filename: "{sys}\notepad.exe"; Parameters: """{app}\README.md"""
Name: "{group}\Uninstall ShinyHarbor"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\shinyharbor.exe"; Parameters: "--config ""{code:DataDirectory}\harbor.toml"""; Description: "Start ShinyHarbor"; Flags: nowait postinstall skipifsilent runasoriginaluser; Check: NotService

[UninstallDelete]
Type: files; Name: "{app}\data-directory.txt"

[InstallDelete]
Type: files; Name: "{group}\Open applications.url"
Type: files; Name: "{group}\Usage dashboard.url"

[Code]
var
  DataPage: TInputDirWizardPage;

function DataDirectory(Param: String): String;
begin
  Result := DataPage.Values[0];
end;

function NotService: Boolean;
begin
  Result := not WizardIsTaskSelected('service');
end;

procedure InitializeWizard;
var DefaultData: String;
begin
  if IsAdminInstallMode then DefaultData := ExpandConstant('{commonappdata}\ShinyHarbor')
  else DefaultData := ExpandConstant('{localappdata}\ShinyHarbor');
  DefaultData := GetPreviousData('DataDirectory', DefaultData);
  DefaultData := ExpandConstant('{param:DATADIR|' + DefaultData + '}');
  DataPage := CreateInputDirPage(wpSelectDir, 'Application data',
    'Where should ShinyHarbor keep your apps and settings?',
    'Choose a local drive or another data drive. This folder is preserved when you upgrade or uninstall. R must be installed separately from cran.r-project.org.', False, '');
  DataPage.Add('Apps, configuration, R packages and usage data:');
  DataPage.Values[0] := DefaultData;
end;

procedure RegisterPreviousData(PreviousDataKey: Integer);
begin
  SetPreviousData(PreviousDataKey, 'DataDirectory', DataDirectory(''));
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var Code: Integer; Script: String;
begin
  Result := '';
  Script := ExpandConstant('{app}\packaging\windows-service.ps1');
  if IsAdminInstallMode and FileExists(Script) then
    if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
      '-NoProfile -ExecutionPolicy Bypass -File "' + Script + '" -Action Stop -Binary "' + ExpandConstant('{app}\shinyharbor.exe') + '"', '', SW_HIDE, ewWaitUntilTerminated, Code) or (Code <> 0) then
        Result := 'Could not stop the existing ShinyHarbor service. Stop it in Services and retry.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var Code: Integer; Args: String; ShowMode: Integer;
begin
  if CurStep = ssPostInstall then begin
    ForceDirectories(DataDirectory(''));
    Args := 'setup --directory "' + DataDirectory('') + '"';
    if WizardIsTaskSelected('shiny') or WizardIsTaskSelected('service') then Args := Args + ' --install-shiny';
    ShowMode := SW_SHOW;
    if WizardSilent then ShowMode := SW_HIDE;
    if not Exec(ExpandConstant('{app}\shinyharbor.exe'), Args, '', ShowMode, ewWaitUntilTerminated, Code) or (Code <> 0) then
      RaiseException('Setup could not configure R/Shiny. Install R from cran.r-project.org and run ShinyHarbor setup again. Your files were preserved.');
    if not SaveStringToFile(ExpandConstant('{app}\data-directory.txt'), UTF8Encode(DataDirectory('')), False) then
      RaiseException('Could not save the application data location.');
    if WizardIsTaskSelected('service') then begin
      Args := '-NoProfile -ExecutionPolicy Bypass -File "' + ExpandConstant('{app}\packaging\windows-service.ps1') + '" -Action Install -Binary "' + ExpandConstant('{app}\shinyharbor.exe') + '" -DataDirectory "' + DataDirectory('') + '"';
      if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'), Args, '', SW_SHOW, ewWaitUntilTerminated, Code) or (Code <> 0) then
        RaiseException('Background service setup failed. See the PowerShell diagnostic and docs/INSTALLATION.md.');
    end;
  end;
end;

function InitializeUninstall: Boolean;
var Code: Integer;
begin
  Result := True;
  if IsAdminInstallMode then
    Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
      '-NoProfile -ExecutionPolicy Bypass -File "' + ExpandConstant('{app}\packaging\windows-service.ps1') + '" -Action Remove -Binary "' + ExpandConstant('{app}\shinyharbor.exe') + '"', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;
