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
Stopped=Setup stopped. Log: %1%nRun Whaybench Setup again to continue.
RemoveLinux=Delete the Whaybench Linux system?%n%nDeleted: conversations, settings, files in the Linux system, ~/work.before-disk, ~/work.unmounted-*%nKept: ~/work (work disk %1)
RemoveLinuxNoDisk=Delete the Whaybench Linux system?%n%nDeleted: conversations, settings, ~/work, all files in the Linux system
TaskLeft=Scheduled task Whaybench\MountWork not removed: administrator rights required.%nTask action: attach %1. Remove: Task Scheduler.

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
  else if Code <> 0 then SuppressibleMsgBox(FmtMessage(CustomMessage('Stopped'), [ExpandConstant('{app}\install.log')]), mbError, MB_OK, IDOK);
end;

function NeedRestart(): Boolean;
begin
  Result := Restart;
end;

// The distribution holds the person's work: deleted only when they say so.
procedure CurUninstallStepChanged(Step: TUninstallStep);
var Code: Integer; Disk, Question: String;
begin
  if Step <> usUninstall then exit;
  RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\RunOnce', 'WhaybenchSetup');
  Disk := ExpandConstant('{localappdata}\Whaybench\work.vhdx');
  // The scheduled task that attaches the work disk; work.vhdx itself stays (the projects outlive the program). Made
  // with administrator rights, it may refuse to go without them: then it is said, and it stays harmless (it attaches
  // the disk, nothing more).
  Exec(ExpandConstant('{sys}\schtasks.exe'), '/delete /tn Whaybench\MountWork /f', '', SW_HIDE, ewWaitUntilTerminated, Code);
  if (Code <> 0) and Exec(ExpandConstant('{sys}\schtasks.exe'), '/query /tn Whaybench\MountWork', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0) then
    SuppressibleMsgBox(FmtMessage(CustomMessage('TaskLeft'), [Disk]), mbInformation, MB_OK, IDOK);
  // ~/work is on the work disk only once it was mounted there (the flag whaybench-work-disk leaves): a work.vhdx that
  // never was leaves ~/work inside the Linux system. Unknown counts as inside.
  if FileExists(Disk) and Exec(ExpandConstant('{sys}\wsl.exe'), '-d Whaybench -u root -- test -f /etc/whaybench/work-on-disk', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0) then
    Question := FmtMessage(CustomMessage('RemoveLinux'), [Disk])
  else Question := CustomMessage('RemoveLinuxNoDisk');
  // A silent uninstall keeps it.
  if SuppressibleMsgBox(Question, mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
    Exec(ExpandConstant('{sys}\wsl.exe'), '--unregister Whaybench', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;
