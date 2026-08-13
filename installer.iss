#define MyAppName "表格合并"
#define MyAppVersion "0.3.0"
#define MyAppPublisher "Wyk521"
#define MyAppExeName "SheetForge.exe"

[Setup]
AppId={{D690101D-223D-4BD8-A2F4-79D4235A86EF}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\表格合并
DefaultGroupName=表格合并
DisableProgramGroupPage=yes
OutputDir=dist
OutputBaseFilename=SheetMerge-Setup
SetupIconFile=assets\app.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "chinesesimp"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加快捷方式："; Flags: unchecked

[Files]
Source: "target\release\SheetForge.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\表格合并"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\表格合并"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "运行表格合并"; Flags: nowait postinstall skipifsilent
