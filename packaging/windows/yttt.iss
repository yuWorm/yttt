#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-local"
#endif
#ifndef SourceBinary
  #define SourceBinary "..\..\target\x86_64-pc-windows-msvc\release\yttt.exe"
#endif
#ifndef SetupIcon
  #define SetupIcon "..\..\assets\app-icon\windows\AppIcon.ico"
#endif
#ifndef OutputDirectory
  #define OutputDirectory "..\..\target\windows"
#endif
#ifndef OutputBaseFilename
  #define OutputBaseFilename "yttt-setup"
#endif

#define MyAppName "yttt"
#define MyAppExeName "yttt.exe"

[Setup]
AppId={{6DDB62D5-C146-4DCC-9942-3177F1228A63}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppName}
DefaultDirName={localappdata}\Programs\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir={#OutputDirectory}
OutputBaseFilename={#OutputBaseFilename}
SetupIconFile={#SetupIcon}
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "foldercontext"; Description: "Add Open with yttt to folder context menus"; GroupDescription: "Explorer integration:"; Flags: unchecked

[Files]
Source: "{#SourceBinary}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{userdocs}"; IconFilename: "{app}\{#MyAppExeName}"; IconIndex: 0
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{userdocs}"; IconFilename: "{app}\{#MyAppExeName}"; IconIndex: 0; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#MyAppExeName}"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExeName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\yttt"; ValueType: string; ValueName: ""; ValueData: "Open with yttt"; Tasks: foldercontext; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\yttt"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName},0"; Tasks: foldercontext
Root: HKA; Subkey: "Software\Classes\Directory\shell\yttt\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: foldercontext
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\yttt"; ValueType: string; ValueName: ""; ValueData: "Open with yttt"; Tasks: foldercontext; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\yttt"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName},0"; Tasks: foldercontext
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\yttt\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%V"""; Tasks: foldercontext

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent
