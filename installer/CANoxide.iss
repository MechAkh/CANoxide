; CANoxide setup script for Inno Setup 6.7.3.
;
; Build with installer\build.ps1, or directly:
;   ISCC.exe /DAppVersion=0.5.0 [/DExeDir=..\target\release] installer\CANoxide.iss
;
; All source paths are relative to this script. The setup exe goes to
; ..\target\installer\CANoxide-<version>-setup.exe.

#ifndef AppVersion
  #error AppVersion is not defined. Pass /DAppVersion=x.y.z to ISCC (build.ps1 does this).
#endif

#ifndef ExeDir
  #define ExeDir "..\target\release"
#endif

#define AppName      "CANoxide"
#define AppPublisher "MechAkh"
#define AppURL       "https://github.com/MechAkh/CANoxide"
#define AppExe       "CANoxide.exe"
#define CliExe       "canoxide-cli.exe"

[Setup]
; Fixed AppId: never change it, or a new version no longer upgrades in place.
AppId={{089894BC-3DF1-4DCF-BB97-241F806CA212}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}
AppUpdatesURL={#AppURL}
AppCopyright=Copyright (c) 2026 {#AppPublisher}
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoProductVersion={#AppVersion}
VersionInfoDescription={#AppName} setup
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
AllowNoIcons=yes
LicenseFile=..\LICENSE
; "Just me" needs no admin rights; "all users" asks for them.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
OutputDir=..\target\installer
OutputBaseFilename={#AppName}-{#AppVersion}-setup
SetupIconFile=..\assets\icon\canoxide.ico
; The app icon: icon group 1 of CANoxide.exe (build.rs).
UninstallDisplayIcon={app}\{#AppExe},0
UninstallDisplayName={#AppName} {#AppVersion}
; Close a running CANoxide before its files are replaced; do not restart it.
CloseApplications=yes
RestartApplications=no
; Tell Explorer and other programs to reload PATH (install and uninstall).
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add canoxide-cli to PATH"; GroupDescription: "Command line:"; Flags: unchecked

[Files]
Source: "{#ExeDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ExeDir}\{#CliExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
; Licences of the crates and fonts in the exes (tools\make_notices.py).
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\docs\CLI.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist
; The marker tells the app to keep its data in %APPDATA%\CANoxide.
Source: "CANoxide.installed"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  UserEnvKey   = 'Environment';
  SystemEnvKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  HKEY_CURRENT_USER_H  = $80000001;
  HKEY_LOCAL_MACHINE_H = $80000002;
  KEY_QUERY_VALUE      = $0001;
  REG_SZ_TYPE          = 1;
  REG_EXPAND_SZ_TYPE   = 2;

function RegOpenKeyExW(hKey: Cardinal; lpSubKey: String; ulOptions: Cardinal;
  samDesired: Cardinal; var phkResult: Cardinal): Integer;
  external 'RegOpenKeyExW@advapi32.dll stdcall';
function RegQueryValueExW(hKey: Cardinal; lpValueName: String; lpReserved: Cardinal;
  var lpType: Cardinal; lpData: Cardinal; var lpcbData: Cardinal): Integer;
  external 'RegQueryValueExW@advapi32.dll stdcall';
function RegCloseKey(hKey: Cardinal): Integer;
  external 'RegCloseKey@advapi32.dll stdcall';

{ The PATH of a per-user install is the user PATH; of an all-users install,
  the system PATH. }
function EnvRoot: Integer;
begin
  if IsAdminInstallMode then
    Result := HKEY_LOCAL_MACHINE
  else
    Result := HKEY_CURRENT_USER;
end;

function EnvKey: String;
begin
  if IsAdminInstallMode then
    Result := SystemEnvKey
  else
    Result := UserEnvKey;
end;

{ Registry type of the Path value (REG_SZ or REG_EXPAND_SZ), so that a
  rewrite keeps it. Default REG_EXPAND_SZ when the value does not exist. }
function PathValueType: Cardinal;
var
  Root, H, VType, Size: Cardinal;
begin
  Result := REG_EXPAND_SZ_TYPE;
  if IsAdminInstallMode then
    Root := HKEY_LOCAL_MACHINE_H
  else
    Root := HKEY_CURRENT_USER_H;
  if RegOpenKeyExW(Root, EnvKey, 0, KEY_QUERY_VALUE, H) = 0 then
  begin
    Size := 0;
    VType := 0;
    if RegQueryValueExW(H, 'Path', 0, VType, 0, Size) = 0 then
      if (VType = REG_SZ_TYPE) or (VType = REG_EXPAND_SZ_TYPE) then
        Result := VType;
    RegCloseKey(H);
  end;
end;

function ReadPath(var Value: String): Boolean;
begin
  Result := RegQueryStringValue(EnvRoot, EnvKey, 'Path', Value);
  if not Result then
    Value := '';
end;

function WritePath(const Value: String; VType: Cardinal): Boolean;
begin
  if VType = REG_SZ_TYPE then
    Result := RegWriteStringValue(EnvRoot, EnvKey, 'Path', Value)
  else
    Result := RegWriteExpandStringValue(EnvRoot, EnvKey, 'Path', Value);
end;

{ Compare folder names without case and without a trailing backslash. }
function SameDir(A, B: String): Boolean;
begin
  A := RemoveBackslashUnlessRoot(Trim(A));
  B := RemoveBackslashUnlessRoot(Trim(B));
  Result := CompareText(A, B) = 0;
end;

{ Find the segment of Path that names Dir. Returns its start (1-based) and
  length, or 0 when no segment matches. }
function FindSegment(const Path, Dir: String; var SegLen: Integer): Integer;
var
  I, Start: Integer;
begin
  Result := 0;
  SegLen := 0;
  Start := 1;
  for I := 1 to Length(Path) + 1 do
  begin
    if (I > Length(Path)) or (Path[I] = ';') then
    begin
      if (I > Start) and SameDir(Copy(Path, Start, I - Start), Dir) then
      begin
        Result := Start;
        SegLen := I - Start;
        Exit;
      end;
      Start := I + 1;
    end;
  end;
end;

procedure AddAppToPath;
var
  Path, Dir: String;
  SegLen: Integer;
  VType: Cardinal;
begin
  Dir := ExpandConstant('{app}');
  VType := PathValueType;
  ReadPath(Path);
  { A second install finds the folder already there: no duplicate. }
  if FindSegment(Path, Dir, SegLen) > 0 then
    Exit;
  if Path = '' then
    Path := Dir
  else if Path[Length(Path)] = ';' then
    Path := Path + Dir + ';'
  else
    Path := Path + ';' + Dir;
  if not WritePath(Path, VType) then
    Log('Could not write the PATH value.');
end;

procedure RemoveAppFromPath;
var
  Path, Dir: String;
  Pos1, SegLen: Integer;
  VType: Cardinal;
  Changed: Boolean;
begin
  Dir := ExpandConstant('{app}');
  if not ReadPath(Path) then
    Exit;
  VType := PathValueType;
  Changed := False;
  Pos1 := FindSegment(Path, Dir, SegLen);
  while Pos1 > 0 do
  begin
    { Remove the segment with one separator: the one before it when there
      is one, else the one after it. This undoes AddAppToPath exactly. }
    if Pos1 > 1 then
      Delete(Path, Pos1 - 1, SegLen + 1)
    else if (SegLen < Length(Path)) and (Path[SegLen + 1] = ';') then
      Delete(Path, 1, SegLen + 1)
    else
      Delete(Path, 1, SegLen);
    Changed := True;
    Pos1 := FindSegment(Path, Dir, SegLen);
  end;
  if Changed then
  begin
    if Path = '' then
      RegDeleteValue(EnvRoot, EnvKey, 'Path')
    else if not WritePath(Path, VType) then
      Log('Could not write the PATH value.');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddAppToPath;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usUninstall then
    RemoveAppFromPath;
  if CurUninstallStep = usPostUninstall then
  begin
    { Keep the user data unless the user says otherwise; a silent uninstall
      always keeps it. }
    DataDir := ExpandConstant('{userappdata}\{#AppName}');
    if (not UninstallSilent) and DirExists(DataDir) then
      if MsgBox('Also delete your settings and device library?' + #13#10#13#10 + DataDir,
                mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;
