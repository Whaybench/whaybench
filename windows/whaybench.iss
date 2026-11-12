; Whaybench Setup for Windows: puts install.ps1, setup-wsl.sh and the icon in the person's AppData and runs install.ps1,
; which brings WSL, the Whaybench distribution and the shortcuts. No administrator rights for itself: WSL asks for them
; when it has to be installed. Build: iscc /DAppVersion=0.5.0 whaybench.iss (gives Output\WhaybenchSetup.exe).
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F4B1E2A-8C3D-4E5F-9A7B-2D1C0E3F4A5B}
AppName=Whaybench
AppVersion={#AppVersion}
AppPublisher=Whaybench
AppPublisherURL=https://github.com/Whaybench/whaybench
DefaultDirName={localappdata}\Whaybench
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
; 64-bit, so the system folder it starts PowerShell and wsl.exe from is the real one.
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
; WSLg, which shows Linux windows, needs Windows 10 21H2 or newer.
MinVersion=10.0.19044
SetupIconFile=whaybench.ico
UninstallDisplayIcon={app}\whaybench.ico
OutputBaseFilename=WhaybenchSetup
WizardStyle=modern
ShowLanguageDialog=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ko"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "ja"; MessagesFile: "compiler:Languages\Japanese.isl"

[Files]
Source: "install.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "setup-wsl.sh"; DestDir: "{app}"; Flags: ignoreversion
Source: "whaybench.ico"; DestDir: "{app}"; Flags: ignoreversion

[UninstallDelete]
; install.ps1 makes the shortcuts; they go with the program.
Type: files; Name: "{userdesktop}\Whaybench.lnk"
Type: files; Name: "{userprograms}\Whaybench.lnk"

[CustomMessages]
Stopped=The setup stopped before it finished; its window showed why. Run Whaybench Setup again to continue from there.
RemoveLinux=Also delete the Whaybench Linux system, with everything kept in it (conversations, settings, files made there)?

[Code]
var Restart: Boolean;

// install.ps1 in a window of its own, where its progress shows. 3010: Windows has to restart once for WSL, and
// install.ps1 continues by itself after it.
procedure CurStepChanged(Step: TSetupStep);
var Code: Integer;
begin
  if Step <> ssPostInstall then exit;
  Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
       ExpandConstant('-NoProfile -ExecutionPolicy Bypass -File "{app}\install.ps1"'), '', SW_SHOW, ewWaitUntilTerminated, Code);
  if Code = 3010 then Restart := True
  else if Code <> 0 then SuppressibleMsgBox(CustomMessage('Stopped'), mbError, MB_OK, IDOK);
end;

function NeedRestart(): Boolean;
begin
  Result := Restart;
end;

// The distribution holds the person's work: deleted only when they say so.
procedure CurUninstallStepChanged(Step: TUninstallStep);
var Code: Integer;
begin
  if Step <> usUninstall then exit;
  RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\RunOnce', 'WhaybenchSetup');
  // A silent uninstall keeps it.
  if SuppressibleMsgBox(CustomMessage('RemoveLinux'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
    Exec(ExpandConstant('{sys}\wsl.exe'), '--unregister Whaybench', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;
