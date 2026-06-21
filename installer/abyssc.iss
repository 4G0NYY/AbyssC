; ============================================================================
;  AbyssC — Inno Setup script
;
;  Installs the CLI (abyssc.exe) and the GUI (abyssc-gui.exe), optionally adds
;  the CLI to PATH, creates Start Menu shortcuts, and registers a cascading
;  "AbyssC" right-click menu on files and folders.
;
;  Build with:  installer\build.ps1   (which injects the version from Cargo.toml)
;  or directly: ISCC /DAppVersion=0.3.0 installer\abyssc.iss
; ============================================================================

#ifndef AppVersion
  #define AppVersion "0.3.0"
#endif

#define MyAppName "AbyssC"
#define MyAppPublisher "4G0NYY"
#define MyCli "abyssc.exe"
#define MyGui "abyssc-gui.exe"

[Setup]
; A stable, unique identity for upgrades/uninstall. Do not change between versions.
AppId={{8F3A9C12-7B4E-4D6A-9E2F-1C5D8A0B3E47}
AppName={#MyAppName}
AppVersion={#AppVersion}
AppVerName={#MyAppName} {#AppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
SetupIconFile=..\abyss_gui\assets\AbyssC.ico
UninstallDisplayIcon={app}\{#MyGui}
OutputDir=dist
OutputBaseFilename=AbyssC-{#AppVersion}-Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Program Files + machine-wide PATH/context-menu need elevation.
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "addtopath";   Description: "Add the &abyssc CLI to the system PATH";              GroupDescription: "Command line:"
Name: "contextmenu"; Description: "Add &AbyssC to the Windows right-click menu";         GroupDescription: "Integration:"
Name: "desktopicon"; Description: "Create a &desktop shortcut";                          GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyCli}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\{#MyGui}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md";               DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\{#MyAppName}";           Filename: "{app}\{#MyGui}"
Name: "{group}\AbyssC Commander";       Filename: "{app}\{#MyGui}"; Parameters: "--browse"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}";     Filename: "{app}\{#MyGui}"; Tasks: desktopicon

[Registry]
; --- PATH (idempotent; see NeedsAddPath) -----------------------------------
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
    ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
    Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}'))

; --- Context menu: cascading "AbyssC" submenu on ALL FILES (*) --------------
Root: HKCR; Subkey: "*\shell\AbyssC"; ValueType: string; ValueName: "MUIVerb"; ValueData: "AbyssC"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCR; Subkey: "*\shell\AbyssC"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyGui}"; Tasks: contextmenu
Root: HKCR; Subkey: "*\shell\AbyssC"; ValueType: string; ValueName: "SubCommands"; ValueData: ""; Tasks: contextmenu
Root: HKCR; Subkey: "*\shell\AbyssC\shell\01compress"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Compress with AbyssC"; Tasks: contextmenu
Root: HKCR; Subkey: "*\shell\AbyssC\shell\01compress\command"; ValueType: string; ValueData: """{app}\{#MyGui}"" --compress ""%1"""; Tasks: contextmenu
Root: HKCR; Subkey: "*\shell\AbyssC\shell\02extract"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Extract with AbyssC"; Tasks: contextmenu
Root: HKCR; Subkey: "*\shell\AbyssC\shell\02extract\command"; ValueType: string; ValueData: """{app}\{#MyGui}"" --extract ""%1"""; Tasks: contextmenu
Root: HKCR; Subkey: "*\shell\AbyssC\shell\03browse"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Open in AbyssC Commander"; Tasks: contextmenu
Root: HKCR; Subkey: "*\shell\AbyssC\shell\03browse\command"; ValueType: string; ValueData: """{app}\{#MyGui}"" --browse ""%1"""; Tasks: contextmenu

; --- Context menu: cascading "AbyssC" submenu on FOLDERS --------------------
Root: HKCR; Subkey: "Directory\shell\AbyssC"; ValueType: string; ValueName: "MUIVerb"; ValueData: "AbyssC"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCR; Subkey: "Directory\shell\AbyssC"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyGui}"; Tasks: contextmenu
Root: HKCR; Subkey: "Directory\shell\AbyssC"; ValueType: string; ValueName: "SubCommands"; ValueData: ""; Tasks: contextmenu
Root: HKCR; Subkey: "Directory\shell\AbyssC\shell\01compress"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Compress with AbyssC"; Tasks: contextmenu
Root: HKCR; Subkey: "Directory\shell\AbyssC\shell\01compress\command"; ValueType: string; ValueData: """{app}\{#MyGui}"" --compress ""%1"""; Tasks: contextmenu
Root: HKCR; Subkey: "Directory\shell\AbyssC\shell\02browse"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Open in AbyssC Commander"; Tasks: contextmenu
Root: HKCR; Subkey: "Directory\shell\AbyssC\shell\02browse\command"; ValueType: string; ValueData: """{app}\{#MyGui}"" --browse ""%1"""; Tasks: contextmenu

; --- Context menu: "Open AbyssC Commander here" on folder BACKGROUND --------
Root: HKCR; Subkey: "Directory\Background\shell\AbyssC"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Open AbyssC Commander here"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCR; Subkey: "Directory\Background\shell\AbyssC"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyGui}"; Tasks: contextmenu
Root: HKCR; Subkey: "Directory\Background\shell\AbyssC\command"; ValueType: string; ValueData: """{app}\{#MyGui}"" --browse ""%V"""; Tasks: contextmenu

[Run]
Filename: "{app}\{#MyGui}"; Description: "Launch {#MyAppName} now"; Flags: nowait postinstall skipifsilent

[Code]
// True only if the given directory is not already on the system PATH.
function NeedsAddPath(Param: string): Boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKLM,
    'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
    'Path', OrigPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Lowercase(Param) + ';', ';' + Lowercase(OrigPath) + ';') = 0;
end;

// On uninstall, strip our entry back out of the system PATH.
procedure RemoveFromPath(Dir: string);
var
  OrigPath, Needle: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKLM,
    'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
    'Path', OrigPath) then
    exit;

  Needle := ';' + Dir;
  P := Pos(Lowercase(Needle), Lowercase(OrigPath));
  if P > 0 then
  begin
    Delete(OrigPath, P, Length(Needle));
    RegWriteExpandStringValue(HKLM,
      'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
      'Path', OrigPath);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
