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
#ifndef InstallerArchitectures
  #define InstallerArchitectures "x64compatible"
#endif
#ifndef InstallerUses64BitInstallMode
  #define InstallerUses64BitInstallMode 1
#endif
#ifndef PostInstallLaunchFlags
  #define PostInstallLaunchFlags "nowait postinstall skipifsilent runasoriginaluser"
#endif

#define MyAppName "yttt"
#define MyAppExeName "yttt.exe"

[Setup]
AppId={{6DDB62D5-C146-4DCC-9942-3177F1228A63}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppName}
DefaultDirName={autopf}\{#MyAppName}
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
ArchitecturesAllowed={#InstallerArchitectures}
#if InstallerUses64BitInstallMode
ArchitecturesInstallIn64BitMode=x64compatible
#endif
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "ChineseSimplified.isl"

[CustomMessages]
english.CreateDesktopShortcut=Create a desktop shortcut
chinesesimplified.CreateDesktopShortcut=创建桌面快捷方式
english.AdditionalShortcuts=Additional shortcuts:
chinesesimplified.AdditionalShortcuts=附加快捷方式：
english.AddFolderContextMenus=Add “Open with yttt” to folder context menus
chinesesimplified.AddFolderContextMenus=在文件夹右键菜单中添加“使用 yttt 打开”
english.ExplorerIntegration=Explorer integration:
chinesesimplified.ExplorerIntegration=资源管理器集成：
english.OpenWithYttt=Open with yttt
chinesesimplified.OpenWithYttt=使用 yttt 打开
english.LaunchYttt=Launch yttt
chinesesimplified.LaunchYttt=启动 yttt
english.UserScope=user
chinesesimplified.UserScope=当前用户
english.MachineScope=machine
chinesesimplified.MachineScope=所有用户
english.CannotReadExistingPath=Cannot read the existing PATH as a string; it was not changed.
chinesesimplified.CannotReadExistingPath=无法将现有 PATH 读取为字符串；未作任何更改。
english.OwnedPathDifferentDirectory=yttt already owns the PATH entry for %1.%nUpgrade that directory, or uninstall yttt before choosing a different destination.
chinesesimplified.OwnedPathDifferentDirectory=yttt 已拥有 %1 的 PATH 条目。%n请升级该目录，或在选择其他目标目录前先卸载 yttt。
english.OwnedPathDifferentScope=yttt already owns a %1 PATH entry for this directory.%nUninstall it before changing the installation scope.
chinesesimplified.OwnedPathDifferentScope=yttt 已拥有此目录的 %1 PATH 条目。%n更改安装范围前请先卸载。
english.CannotDetermineUserProfile=Cannot determine the installing user profile; PATH registration was not completed.
chinesesimplified.CannotDetermineUserProfile=无法确定安装用户的配置文件；未完成 PATH 注册。
english.CannotRecordInstallScope=Cannot record the installation scope; PATH registration was not completed.
chinesesimplified.CannotRecordInstallScope=无法记录安装范围；未完成 PATH 注册。
english.InvalidInstallerState=The selected directory contains invalid yttt installer state.%nRestore the original yttt-installer-state.ini before upgrading or uninstalling.
chinesesimplified.InvalidInstallerState=所选目录包含无效的 yttt 安装程序状态。%n升级或卸载前，请还原原始 yttt-installer-state.ini。
english.SelectedDirectoryOtherScope=The selected directory belongs to a %1 yttt installation.%nUninstall it before changing the installation scope.
chinesesimplified.SelectedDirectoryOtherScope=所选目录属于 %1 yttt 安装。%n更改安装范围前请先卸载。
english.SelectedDirectoryOtherUser=The selected directory belongs to another user's yttt installation.%nUninstall it as that user before reusing this directory.
chinesesimplified.SelectedDirectoryOtherUser=所选目录属于其他用户的 yttt 安装。%n重新使用此目录前，请以该用户身份卸载。
english.InvalidScopeRecord=The yttt installation scope record is missing or invalid. Restore the original yttt-installer-state.ini before uninstalling.
chinesesimplified.InvalidScopeRecord=yttt 安装范围记录缺失或无效。卸载前，请还原原始 yttt-installer-state.ini。
english.OtherUserInstallation=This is another user's yttt installation. Run its uninstaller as the original installing user.
chinesesimplified.OtherUserInstallation=这是其他用户的 yttt 安装。请以最初安装的用户身份运行其卸载程序。
english.DirectoryContainsSemicolon=The selected installation directory contains a semicolon.%nChoose a directory without semicolons because it cannot be registered in PATH.
chinesesimplified.DirectoryContainsSemicolon=所选安装目录包含分号。%n请选择不含分号的目录，因为它无法注册到 PATH 中。
english.CannotDetermineInstallingProfile=Cannot determine the installing user profile needed to safely maintain PATH.
chinesesimplified.CannotDetermineInstallingProfile=无法确定安全维护 PATH 所需的安装用户配置文件。
english.CannotRunHostPreflight=Unable to run the installed yttt Host upgrade preflight.
chinesesimplified.CannotRunHostPreflight=无法运行已安装 yttt Host 的升级预检。
english.HostPreflightNoDetails=The installed yttt Host did not provide blocker details.
chinesesimplified.HostPreflightNoDetails=已安装的 yttt Host 未提供阻塞原因。
english.HostResourcesBlockUpgrade=yttt cannot be upgraded while the Host owns running or unacknowledged resources.%n%1
chinesesimplified.HostResourcesBlockUpgrade=Host 正在占用运行中或未确认的资源，因此无法升级 yttt。%n%1
english.HostPreflightFailed=yttt Host upgrade preflight failed.%n%1
chinesesimplified.HostPreflightFailed=yttt Host 升级预检失败。%n%1

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopShortcut}"; GroupDescription: "{cm:AdditionalShortcuts}"; Flags: unchecked
Name: "foldercontext"; Description: "{cm:AddFolderContextMenus}"; GroupDescription: "{cm:ExplorerIntegration}"; Flags: unchecked

[Files]
Source: "{#SourceBinary}"; DestDir: "{app}"; Flags: ignoreversion; AfterInstall: WriteInstallerState

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{userdocs}"; IconFilename: "{app}\{#MyAppExeName}"; IconIndex: 0
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{userdocs}"; IconFilename: "{app}\{#MyAppExeName}"; IconIndex: 0; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#MyAppExeName}"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExeName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\yttt"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWithYttt}"; Tasks: foldercontext; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\yttt"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName},0"; Tasks: foldercontext
Root: HKA; Subkey: "Software\Classes\Directory\shell\yttt\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: foldercontext
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\yttt"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWithYttt}"; Tasks: foldercontext; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\yttt"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName},0"; Tasks: foldercontext
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\yttt\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%V"""; Tasks: foldercontext

Root: HKA; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{code:PathValueForInstall|user}"; Flags: preservestringtype; Check: not IsAdminInstallMode
Root: HKA; Subkey: "Software\yttt\Installer"; ValueType: string; ValueName: "InstallPath"; ValueData: "{app}"; Check: not IsAdminInstallMode
Root: HKA; Subkey: "Software\yttt\Installer"; ValueType: string; ValueName: "PathEntry"; ValueData: "{code:PathOwnershipEntryForInstall|user}"; Check: not IsAdminInstallMode
Root: HKA; Subkey: "Software\yttt\Installer"; ValueType: string; ValueName: "PathBefore"; ValueData: "{code:PathOwnershipBeforeForInstall|user}"; Check: not IsAdminInstallMode
Root: HKA; Subkey: "Software\yttt\Installer"; ValueType: dword; ValueName: "PathExisted"; ValueData: "{code:PathOwnershipExistedForInstall|user}"; Check: not IsAdminInstallMode
Root: HKA; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{code:PathValueForInstall|machine}"; Flags: preservestringtype; Check: IsAdminInstallMode
Root: HKA; Subkey: "Software\yttt\Installer"; ValueType: string; ValueName: "InstallPath"; ValueData: "{app}"; Check: IsAdminInstallMode
Root: HKA; Subkey: "Software\yttt\Installer"; ValueType: string; ValueName: "PathEntry"; ValueData: "{code:PathOwnershipEntryForInstall|machine}"; Check: IsAdminInstallMode
Root: HKA; Subkey: "Software\yttt\Installer"; ValueType: string; ValueName: "PathBefore"; ValueData: "{code:PathOwnershipBeforeForInstall|machine}"; Check: IsAdminInstallMode
Root: HKA; Subkey: "Software\yttt\Installer"; ValueType: dword; ValueName: "PathExisted"; ValueData: "{code:PathOwnershipExistedForInstall|machine}"; Check: IsAdminInstallMode

[UninstallDelete]
Type: files; Name: "{app}\yttt-installer-state.ini"


[Code]

const
  UserEnvironmentKey = 'Environment';
  MachineEnvironmentKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  PathOwnershipKey = 'Software\yttt\Installer';
  PathEntryValue = 'PathEntry';
  InstallPathValue = 'InstallPath';
  PathBeforeValue = 'PathBefore';
  PathExistedValue = 'PathExisted';
  InstallerStateFileName = 'yttt-installer-state.ini';
  REG_SZ = 1;
  REG_EXPAND_SZ = 2;
  RRF_RT_REG_SZ = $00000002;
  RRF_RT_REG_EXPAND_SZ = $00000004;
  RRF_NOEXPAND = $10000000;
  WM_SETTINGCHANGE = $001A;
  SMTO_ABORTIFHUNG = $0002;

function RegGetValue(
  hKey: Integer; lpSubKey, lpValue: String; dwFlags: Cardinal;
  var pdwType: Cardinal; pvData: Cardinal; var pcbData: Cardinal): Longint;
  external 'RegGetValueW@advapi32.dll stdcall';
function SendMessageTimeout(
  hWnd: HWND; Msg: Cardinal; wParam: Cardinal; lParam: String;
  fuFlags, uTimeout: Cardinal; var lpdwResult: Cardinal): Longint;
  external 'SendMessageTimeoutW@user32.dll stdcall';

var
  UserPathPrepared: Boolean;
  UserPathOwned: Boolean;
  UserPathWasAdded: Boolean;
  UserPathExisted: Boolean;
  UserPathValue: String;
  UserPathBefore: String;
  MachinePathPrepared: Boolean;
  MachinePathOwned: Boolean;
  MachinePathWasAdded: Boolean;
  MachinePathExisted: Boolean;
  MachinePathValue: String;
  MachinePathBefore: String;

function ApplicationPath: String;
begin
  Result := ExpandConstant('{app}');
end;

function InstallerStatePath: String;
begin
  Result := AddBackslash(ApplicationPath) + InstallerStateFileName;
end;

function CurrentInstallScope: String;
begin
  if IsAdminInstallMode then
    Result := 'machine'
  else
    Result := 'user';
end;

function DisplayInstallScope(const Scope: String): String;
begin
  if SameText(Scope, 'machine') then
    Result := CustomMessage('MachineScope')
  else
    Result := CustomMessage('UserScope');
end;

function CurrentUserProfile: String;
begin
  Result := ExpandConstant('{%USERPROFILE|}');
end;

function PathContainsEntry(const PathValue, Entry: String): Boolean;
var
  Entries: TArrayOfString;
  Index: Integer;
begin
  Entries := StringSplit(PathValue, [';'], stAll);
  for Index := 0 to GetArrayLength(Entries) - 1 do
  begin
    if SameText(Entries[Index], Entry) then
    begin
      Result := True;
      exit;
    end;
  end;
  Result := False;
end;

function AppendPathEntry(const PathValue, Entry: String): String;
begin
  if PathValue = '' then
    Result := Entry
  else
    Result := PathValue + ';' + Entry;
end;

function ReadPathOwnership(
  Root: Integer; const OwnershipKey, Entry: String; var OriginalPath: String;
  var OriginalPathExisted: Boolean): Boolean;
var
  OwnedEntry: String;
  ExistedValue: Cardinal;
begin
  Result :=
    RegQueryStringValue(Root, OwnershipKey, PathEntryValue, OwnedEntry) and
    SameText(OwnedEntry, Entry) and
    RegQueryStringValue(Root, OwnershipKey, PathBeforeValue, OriginalPath) and
    RegQueryDWordValue(Root, OwnershipKey, PathExistedValue, ExistedValue);
  if Result then
    OriginalPathExisted := ExistedValue <> 0;
end;

procedure PreparePathForInstall(
  Root: Integer; const EnvironmentKey: String; var Prepared, Owned, WasAdded,
  PathExisted: Boolean; var PathValue, PathBefore: String);
var
  ExistingOwnership: Boolean;
  CurrentPath: String;
begin
  if Prepared then
    exit;

  Prepared := True;
  PathExisted := RegQueryStringValue(Root, EnvironmentKey, 'Path', CurrentPath);
  if not PathExisted and RegValueExists(Root, EnvironmentKey, 'Path') then
    RaiseException(CustomMessage('CannotReadExistingPath'));
  if not PathExisted then
    CurrentPath := '';

  ExistingOwnership := ReadPathOwnership(
    Root, PathOwnershipKey, ApplicationPath, PathBefore, PathExisted);
  Owned := ExistingOwnership;
  WasAdded := False;

  if not PathContainsEntry(CurrentPath, ApplicationPath) then
  begin
    PathBefore := CurrentPath;
    PathExisted := RegQueryStringValue(Root, EnvironmentKey, 'Path', CurrentPath);
    if not PathExisted then
      CurrentPath := '';
    PathValue := AppendPathEntry(CurrentPath, ApplicationPath);
    Owned := True;
    WasAdded := True;
  end
  else
  begin
    PathValue := CurrentPath;
  end;
end;

function ReadOwnedInstallationPath(
  Root: Integer; const EnvironmentKey: String; var InstalledPath: String): Boolean;
var
  CurrentPath: String;
  OriginalPath: String;
  OriginalPathExisted: Boolean;
begin
  Result :=
    RegQueryStringValue(Root, PathOwnershipKey, InstallPathValue, InstalledPath) and
    ReadPathOwnership(
      Root, PathOwnershipKey, InstalledPath, OriginalPath, OriginalPathExisted) and
    RegQueryStringValue(Root, EnvironmentKey, 'Path', CurrentPath) and
    PathContainsEntry(CurrentPath, InstalledPath);
end;

function OwnedPathConflict(
  Root: Integer; const EnvironmentKey, ExistingScope: String): String;
var
  InstalledPath: String;
begin
  Result := '';
  if not ReadOwnedInstallationPath(Root, EnvironmentKey, InstalledPath) then
    exit;

  if not SameText(InstalledPath, ApplicationPath) then
  begin
    Result := FmtMessage(
      CustomMessage('OwnedPathDifferentDirectory'), [AddQuotes(InstalledPath)]);
    exit;
  end;

  if not SameText(ExistingScope, CurrentInstallScope) then
  begin
    Result := FmtMessage(
      CustomMessage('OwnedPathDifferentScope'), [DisplayInstallScope(ExistingScope)]);
  end;
end;

function CheckForOwnedPathConflicts: String;
begin
  Result := OwnedPathConflict(
    HKEY_CURRENT_USER, UserEnvironmentKey, 'user');
  if Result = '' then
    Result := OwnedPathConflict(
      HKEY_LOCAL_MACHINE, MachineEnvironmentKey, 'machine');
end;

procedure PrepareUserPathForInstall;
begin
  PreparePathForInstall(
    HKEY_CURRENT_USER, UserEnvironmentKey, UserPathPrepared, UserPathOwned,
    UserPathWasAdded, UserPathExisted, UserPathValue, UserPathBefore);
end;

procedure PrepareMachinePathForInstall;
begin
  PreparePathForInstall(
    HKEY_LOCAL_MACHINE, MachineEnvironmentKey, MachinePathPrepared,
    MachinePathOwned, MachinePathWasAdded, MachinePathExisted,
    MachinePathValue, MachinePathBefore);
end;

procedure PreparePathForInstallScope(Param: String);
begin
  if SameText(Param, 'machine') then
    PrepareMachinePathForInstall
  else
    PrepareUserPathForInstall;
end;

function PathValueForInstall(Param: String): String;
begin
  PreparePathForInstallScope(Param);
  if SameText(Param, 'machine') then
    Result := MachinePathValue
  else
    Result := UserPathValue;
end;

function PathOwnershipEntryForInstall(Param: String): String;
begin
  PreparePathForInstallScope(Param);
  if (SameText(Param, 'machine') and MachinePathOwned) or
     (not SameText(Param, 'machine') and UserPathOwned) then
    Result := ApplicationPath
  else
    Result := '';
end;

function PathOwnershipBeforeForInstall(Param: String): String;
begin
  PreparePathForInstallScope(Param);
  if SameText(Param, 'machine') then
    Result := MachinePathBefore
  else
    Result := UserPathBefore;
end;

function PathOwnershipExistedForInstall(Param: String): String;
begin
  PreparePathForInstallScope(Param);
  if SameText(Param, 'machine') then
  begin
    if MachinePathExisted then
      Result := '1'
    else
      Result := '0';
  end
  else
  begin
    if UserPathExisted then
      Result := '1'
    else
      Result := '0';
  end;
end;

function GetRegistryStringValueType(
  Root: Integer; const SubKeyName, ValueName: String;
  var ValueType: Cardinal): Boolean;
var
  DataSize: Cardinal;
begin
  DataSize := 0;
  ValueType := 0;
  Result := RegGetValue(
    Root, SubKeyName, ValueName,
    RRF_RT_REG_SZ or RRF_RT_REG_EXPAND_SZ or RRF_NOEXPAND,
    ValueType, 0, DataSize) = 0;
end;

function WritePathPreservingType(
  Root: Integer; const EnvironmentKey, PathValue: String): Boolean;
var
  ValueType: Cardinal;
begin
  Result := False;
  if not GetRegistryStringValueType(
    Root, EnvironmentKey, 'Path', ValueType) then
    exit;

  if ValueType = REG_SZ then
    Result := RegWriteStringValue(Root, EnvironmentKey, 'Path', PathValue)
  else if ValueType = REG_EXPAND_SZ then
    Result := RegWriteExpandStringValue(Root, EnvironmentKey, 'Path', PathValue);
end;

procedure DeletePathOwnership(Root: Integer; const OwnershipKey: String);
begin
  RegDeleteValue(Root, OwnershipKey, PathEntryValue);
  RegDeleteValue(Root, OwnershipKey, InstallPathValue);
  RegDeleteValue(Root, OwnershipKey, PathBeforeValue);
  RegDeleteValue(Root, OwnershipKey, PathExistedValue);
  RegDeleteKeyIfEmpty(Root, OwnershipKey);
end;

function RegisteredPathHasSuffix(
  const CurrentPath, RegisteredPath: String): Boolean;
begin
  Result :=
    (Length(CurrentPath) > Length(RegisteredPath)) and
    SameText(Copy(CurrentPath, 1, Length(RegisteredPath)), RegisteredPath) and
    (CurrentPath[Length(RegisteredPath) + 1] = ';');
end;

function UnregisterOwnedPath(
  Root: Integer; const EnvironmentKey, OwnershipKey: String): Boolean;
var
  CurrentPath: String;
  InstalledPath: String;
  OwnedEntry: String;
  OriginalPath: String;
  OriginalPathExisted: Boolean;
  RegisteredPath: String;
  UpdatedPath: String;
begin
  Result := False;
  if not RegQueryStringValue(
    Root, OwnershipKey, InstallPathValue, InstalledPath) or
     not SameText(InstalledPath, ApplicationPath) then
    exit;
  if not ReadPathOwnership(
    Root, OwnershipKey, ApplicationPath, OriginalPath, OriginalPathExisted) then
  begin
    DeletePathOwnership(Root, OwnershipKey);
    exit;
  end;

  if not RegQueryStringValue(Root, EnvironmentKey, 'Path', CurrentPath) then
  begin
    DeletePathOwnership(Root, OwnershipKey);
    exit;
  end;

  OwnedEntry := ApplicationPath;
  RegisteredPath := AppendPathEntry(OriginalPath, OwnedEntry);
  if SameText(CurrentPath, RegisteredPath) then
  begin
    if OriginalPathExisted then
      Result := WritePathPreservingType(Root, EnvironmentKey, OriginalPath)
    else
      Result := RegDeleteValue(Root, EnvironmentKey, 'Path');
    if Result then
      DeletePathOwnership(Root, OwnershipKey);
    exit;
  end;

  if RegisteredPathHasSuffix(CurrentPath, RegisteredPath) then
  begin
    if OriginalPath = '' then
      UpdatedPath := Copy(
        CurrentPath, Length(RegisteredPath) + 2, Length(CurrentPath))
    else
      UpdatedPath :=
        OriginalPath + Copy(
          CurrentPath, Length(RegisteredPath) + 1, Length(CurrentPath));
    Result := WritePathPreservingType(Root, EnvironmentKey, UpdatedPath);
    if Result then
      DeletePathOwnership(Root, OwnershipKey);
    exit;
  end;

  DeletePathOwnership(Root, OwnershipKey);
end;

procedure BroadcastEnvironmentChange;
var
  MessageResult: Cardinal;
begin
  MessageResult := 0;
  MessageResult := SendMessageTimeout(
    HWND_BROADCAST, WM_SETTINGCHANGE, 0, 'Environment', SMTO_ABORTIFHUNG,
    5000, MessageResult);
end;

function ReadInstallerState(var Scope, UserProfile: String): Boolean;
var
  Lines: TArrayOfString;
  Index: Integer;
  Line: String;
  FormatSeen: Boolean;
  ScopeSeen: Boolean;
  UserProfileSeen: Boolean;
begin
  Result := False;
  Scope := '';
  UserProfile := '';
  if not LoadStringsFromFile(InstallerStatePath, Lines) then
    exit;

  FormatSeen := False;
  ScopeSeen := False;
  UserProfileSeen := False;
  for Index := 0 to GetArrayLength(Lines) - 1 do
  begin
    Line := Lines[Index];
    if Line <> '' then
    begin
      if Line = 'format=1' then
      begin
        if FormatSeen then
          exit;
        FormatSeen := True;
      end
      else if Copy(Line, 1, 6) = 'scope=' then
      begin
        if ScopeSeen then
          exit;
        Scope := Copy(Line, 7, Length(Line));
        ScopeSeen := True;
      end
      else if Copy(Line, 1, 13) = 'user-profile=' then
      begin
        if UserProfileSeen then
          exit;
        UserProfile := Copy(Line, 14, Length(Line));
        UserProfileSeen := True;
      end
      else
        exit;
    end;
  end;

  if not FormatSeen or not ScopeSeen then
    exit;
  if SameText(Scope, 'machine') then
    Result := not UserProfileSeen
  else if SameText(Scope, 'user') then
    Result := UserProfileSeen and (UserProfile <> '');
end;

procedure WriteInstallerState;
var
  Lines: TArrayOfString;
  UserProfile: String;
begin
  SetArrayLength(Lines, 2);
  Lines[0] := 'format=1';
  Lines[1] := 'scope=' + CurrentInstallScope;
  if not IsAdminInstallMode then
  begin
    UserProfile := CurrentUserProfile;
    if UserProfile = '' then
      RaiseException(CustomMessage('CannotDetermineUserProfile'));
    SetArrayLength(Lines, 3);
    Lines[2] := 'user-profile=' + UserProfile;
  end;

  if not SaveStringsToUTF8File(InstallerStatePath, Lines, False) then
    RaiseException(CustomMessage('CannotRecordInstallScope'));
end;

function CheckInstallerStateForInstall: String;
var
  Scope: String;
  UserProfile: String;
  CurrentProfile: String;
begin
  Result := '';
  if not FileExists(InstallerStatePath) then
    exit;
  if not ReadInstallerState(Scope, UserProfile) then
  begin
    Result := CustomMessage('InvalidInstallerState');
    exit;
  end;
  if not SameText(Scope, CurrentInstallScope) then
  begin
    Result := FmtMessage(
      CustomMessage('SelectedDirectoryOtherScope'), [DisplayInstallScope(Scope)]);
    exit;
  end;
  if SameText(Scope, 'user') then
  begin
    CurrentProfile := CurrentUserProfile;
    if (CurrentProfile = '') or not SameText(UserProfile, CurrentProfile) then
      Result := CustomMessage('SelectedDirectoryOtherUser');
  end;
end;

function UnregisterInstalledPath: Boolean;
var
  Scope: String;
  UserProfile: String;
begin
  Result := False;
  if not ReadInstallerState(Scope, UserProfile) then
    exit;

  if SameText(Scope, 'machine') then
    Result := UnregisterOwnedPath(
      HKEY_LOCAL_MACHINE, MachineEnvironmentKey, PathOwnershipKey)
  else if SameText(Scope, 'user') and
          (UserProfile <> '') and
          SameText(UserProfile, CurrentUserProfile) then
    Result := UnregisterOwnedPath(
      HKEY_CURRENT_USER, UserEnvironmentKey, PathOwnershipKey);
end;

function InitializeUninstall: Boolean;
var
  Scope: String;
  UserProfile: String;
begin
  Result := ReadInstallerState(Scope, UserProfile);
  if not Result then
    SuppressibleMsgBox(
      CustomMessage('InvalidScopeRecord'),
      mbError, MB_OK, IDOK)
  else if SameText(Scope, 'user') and
          not SameText(UserProfile, CurrentUserProfile) then
  begin
    Result := False;
    SuppressibleMsgBox(
      CustomMessage('OtherUserInstallation'),
      mbError, MB_OK, IDOK);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  InstalledBinary: String;
  ReportFile: String;
  Detail: AnsiString;
  DetailText: String;
  ResultCode: Integer;
begin
  Result := '';
  if Pos(';', ApplicationPath) <> 0 then
  begin
    Result := CustomMessage('DirectoryContainsSemicolon');
    exit;
  end;

  if (not IsAdminInstallMode) and (CurrentUserProfile = '') then
  begin
    Result := CustomMessage('CannotDetermineInstallingProfile');
    exit;
  end;

  Result := CheckInstallerStateForInstall;
  if Result <> '' then
    exit;
  Result := CheckForOwnedPathConflicts;
  if Result <> '' then
    exit;

  InstalledBinary := ExpandConstant('{app}\{#MyAppExeName}');
  if FileExists(InstalledBinary) then
  begin
    ReportFile := ExpandConstant('{tmp}\yttt-host-preflight.txt');
    DeleteFile(ReportFile);
    if not Exec(
      InstalledBinary,
      '--installer-host-preflight ' + AddQuotes(ReportFile),
      '',
      SW_HIDE,
      ewWaitUntilTerminated,
      ResultCode)
    then
    begin
      Result := CustomMessage('CannotRunHostPreflight');
      exit;
    end;

    if ResultCode <> 0 then
    begin
      if LoadStringFromFile(ReportFile, Detail) then
        DetailText := Trim(Detail)
      else
        DetailText := CustomMessage('HostPreflightNoDetails');
      if ResultCode = 2 then
        Result := FmtMessage(
          CustomMessage('HostResourcesBlockUpgrade'), [DetailText])
      else
        Result := FmtMessage(
          CustomMessage('HostPreflightFailed'), [DetailText]);
      exit;
    end;
  end;

  if IsAdminInstallMode then
    PrepareMachinePathForInstall
  else
    PrepareUserPathForInstall;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and
     ((IsAdminInstallMode and MachinePathWasAdded) or
      (not IsAdminInstallMode and UserPathWasAdded)) then
    BroadcastEnvironmentChange;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  EnvironmentChanged: Boolean;
begin
  if CurUninstallStep <> usUninstall then
    exit;

  EnvironmentChanged := UnregisterInstalledPath;
  if EnvironmentChanged then
    BroadcastEnvironmentChange;
end;

[Run]
Filename: "{cmd}"; Parameters: "/d /v:on /s /c ""set ""PATH=!CD!;!PATH!"" && start """" /b "".\{#MyAppExeName}"""""; WorkingDir: "{app}"; Description: "{cm:LaunchYttt}"; Flags: {#PostInstallLaunchFlags}
