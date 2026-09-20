; Inno Setup script for the Windows installer.
;
; Per user on purpose: the program is a single executable that needs no
; services and no drivers, so asking for administrator rights would buy
; nothing. {autopf} therefore resolves to %LOCALAPPDATA%\Programs, which is
; where per user programs belong on Windows.
;
; Build with:  iscc /DAppVersion=0.1.0 visura.iss
; Expects Visura.exe next to this file.

#define AppName "Visura"
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Never change AppId: it is what lets a new release replace an old one
; instead of installing a second copy beside it.
AppId={{4C6F2A18-9D3E-4B77-9E21-5A0C8B3F71D4}
AppName={#AppName}
AppVersion={#AppVersion}
VersionInfoVersion={#AppVersion}
AppPublisher={#AppName}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
OutputBaseFilename=VisuraSetup
OutputDir=.
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\Visura.exe
LicenseFile=..\LICENSE
; The running program holds this mutex, so the installer can offer to close it
; rather than failing on a locked file.
AppMutex=Local\VisuraSingleInstance
CloseApplications=yes
RestartApplications=no

[Files]
Source: "Visura.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\Visura.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\Visura.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Run]
Filename: "{app}\Visura.exe"; Description: "Start {#AppName}"; \
  Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Thumbnails are rebuilt on demand and mean nothing without the program.
; Settings live in %APPDATA% and screenshots in the pictures folder; both are
; the user's own data and are left alone.
Type: filesandordirs; Name: "{localappdata}\{#AppName}"

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    { Otherwise Windows would keep starting a program that is no longer there. }
    RegDeleteValue(HKEY_CURRENT_USER,
      'Software\Microsoft\Windows\CurrentVersion\Run', '{#AppName}');
end;
