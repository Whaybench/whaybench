# Installs Whaybench on Windows: WSL if it is missing, a Fedora distribution of its own named Whaybench, everything
# inside it (setup-wsl.sh), and shortcuts that start it without a console window. Run it again to finish an install that
# stopped (after the restart WSL may ask for, for one): every step skips what is already there.
#   powershell -ExecutionPolicy Bypass -File install.ps1
param(
    [string]$Distro = "Whaybench",
    # The Fedora release the development machine runs, so what is tested there is what runs here.
    [string]$Fedora = "FedoraLinux-44",
    [string]$Setup = "https://github.com/Whaybench/whaybench/releases/latest/download/setup-wsl.sh",
    [string]$Icon = "https://github.com/Whaybench/whaybench/releases/latest/download/whaybench.ico"
)
$ErrorActionPreference = "Stop"
# Whaybench is built for x86_64 only: on ARM64 Windows nothing is installed.
# The machine's own, not this process's (an x64 PowerShell emulated on ARM64 says AMD64 for itself).
$arch = try { [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() } catch { $env:PROCESSOR_ARCHITECTURE }
if ($arch -notin @("X64", "AMD64")) { Write-Host "Whaybench runs on x86-64 Windows only, not $arch."; exit 1 }
# What this window says is kept in a log, which Whaybench Setup points to when this stops.
# shortcut: Windows PowerShell 5.1 may leave the output of wsl.exe and setup-wsl.sh out of the transcript; the window
# still shows it while it waits. Upgrade if a log without the Linux part proves not enough.
$log = Join-Path $env:LOCALAPPDATA "Whaybench\install.log"
New-Item -ItemType Directory -Force (Split-Path $log) | Out-Null
try { Start-Transcript -Path $log -Append | Out-Null } catch {}
function Say($text) { Write-Host "Whaybench: $text" }
# The window stays until Enter is pressed, so the reason can be read before it closes.
function Fail($text) {
    Write-Host "Whaybench: $text" -ForegroundColor Red
    Write-Host "Whaybench: this is kept in $log"
    try { Stop-Transcript | Out-Null } catch {}
    Read-Host "Press Enter to close this window" | Out-Null
    exit 1
}
# Any other error stops the same way, with its message.
trap { Fail "$_" }
# wsl.exe writes UTF-16; read it as such.
[Console]::OutputEncoding = [System.Text.Encoding]::Unicode
$wsl = "$env:SystemRoot\System32\wsl.exe"

# WSLg, which shows Linux windows, needs Windows 11 or Windows 10 21H2 (build 19044) with WSL from the Store.
$build = [int](Get-CimInstance Win32_OperatingSystem).BuildNumber
if ($build -lt 19044) { Fail "Windows 10 21H2 or newer required (this: build $build)." }

# WSL 2 runs in a virtual machine. Off in the BIOS, WSL still installs and the distribution fails later (0x80370102),
# so it is asked here, before the restart. A running hypervisor (WSL 2, Hyper-V) hides the firmware flag: either is enough.
$hypervisor = (Get-CimInstance Win32_ComputerSystem).HypervisorPresent
$firmware = @(Get-CimInstance Win32_Processor | Where-Object { $_.VirtualizationFirmwareEnabled }).Count -gt 0
if (-not $hypervisor -and -not $firmware) {
    Fail "Virtualization off. Turn on in BIOS/UEFI (Intel VT-x, AMD-V, SVM), then run Setup again."
}

# WSL itself. Without it, wsl.exe --status fails or says it is not installed.
& $wsl --status *> $null
if ($LASTEXITCODE -ne 0) {
    Say "Installing WSL (administrator rights requested)"
    & $wsl --install --no-distribution
    if ($LASTEXITCODE -ne 0) {
        Fail "WSL not installed. Virtualization error: turn on in BIOS/UEFI (Intel VT-x, AMD-V, SVM). Then run Setup again."
    }
    # A first WSL install usually needs a restart; this runs again by itself once Windows is back.
    $again = "powershell -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    Set-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce" -Name "WhaybenchSetup" -Value $again
    Say "Restart Windows to finish. Setup continues after the restart."
    try { Stop-Transcript | Out-Null } catch {}
    exit 3010
}
& $wsl --update *> $null
# Fedora installs by name from WSL 2.4.4 on; an older WSL would fail later with a message about the network.
$ver = (& $wsl --version) -join " "
if ($LASTEXITCODE -ne 0 -or $ver -notmatch '(\d+)\.(\d+)\.(\d+)' -or [version]"$($matches[1]).$($matches[2]).$($matches[3])" -lt [version]"2.4.4") {
    Fail "WSL 2.4.4 or newer required. Update: 'wsl --update' (administrator). Then run Setup again."
}

# The distribution of its own: the person's other Linux distributions are not touched.
$have = (& $wsl --list --quiet) -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ -eq $Distro }
if (-not $have) {
    Say "Installing $Fedora as $Distro (a few minutes)"
    & $wsl --install $Fedora --name $Distro --no-launch
    if ($LASTEXITCODE -ne 0) { Fail "Fedora not installed. Check: internet connection, virtualization (BIOS). Then run Setup again." }
}

# Inside: an account named after the Windows one, the programs, the input method for the Windows display language.
$user = ($env:USERNAME.ToLower() -replace '[^a-z0-9_-]', '')
if ($user -notmatch '^[a-z_]') { $user = "whaybench" }
$lang = (Get-UICulture).TwoLetterISOLanguageName
# Its locale as Linux names it (ko-KR: ko_KR): the input method's own texts come in that language.
$locale = (Get-UICulture).Name -replace '-', '_'
if ($locale -notmatch '^[a-z]{2,3}_[A-Z]{2}$') { $locale = "" }
Say "Setting up $Distro for ${user}: packages, Claude Code, Whaybench (several minutes)"
$script = Join-Path $env:TEMP "whaybench-setup-wsl.sh"
if (Test-Path (Join-Path $PSScriptRoot "setup-wsl.sh")) { Copy-Item (Join-Path $PSScriptRoot "setup-wsl.sh") $script -Force }
else { Invoke-WebRequest -UseBasicParsing -Uri $Setup -OutFile $script }
# Line ends as Linux reads them. The distribution reads the file where it is, from its folder (--cd takes a Windows
# path): text piped from PowerShell gets a Windows line end added, which the shell would run as a command, and
# Windows PowerShell breaks quotes inside arguments to wsl.exe.
[IO.File]::WriteAllText($script, ([IO.File]::ReadAllText($script) -replace "`r`n", "`n"))
& $wsl -d $Distro -u root --cd (Split-Path $script) -- sh (Split-Path -Leaf $script) $user $lang $locale
if ($LASTEXITCODE -ne 0) { Fail "Setup in $Distro stopped. Run Setup again to continue." }
# systemd and the default account apply from the next start of the distribution.
& $wsl --terminate $Distro *> $null

# The work disk: ~/work on work.vhdx beside the distribution, so the projects stay when Whaybench or the distribution
# is removed, and on btrfs, so working copies share their data. Attaching a disk to WSL needs administrator rights:
# once here (Windows asks), and at each start through the scheduled task Whaybench\MountWork, which only attaches the
# disk: nothing runs inside the distribution with those rights (an agent can be root there). Mounting it on ~/work is
# done without them, by whaybench-work-disk as the distribution's root. An existing work.vhdx (an earlier install) is
# attached as it is. Without it Whaybench runs on the distribution's disk.
$wbDir = Join-Path $env:LOCALAPPDATA "Whaybench"
New-Item -ItemType Directory -Force $wbDir | Out-Null
$vhd = Join-Path $wbDir "work.vhdx"
# A native program's error text is no reason to stop here: its exit code says what happened.
function Run-Native([string]$exe, [string[]]$arguments) {
    $ErrorActionPreference = "Continue"
    # The Linux side writes UTF-8, and so does wsl.exe with WSL_UTF8: the log stays readable.
    $was = [Console]::OutputEncoding
    [Console]::OutputEncoding = [Text.Encoding]::UTF8
    $env:WSL_UTF8 = "1"
    & $exe @arguments 2>&1 | Out-File -Append -Encoding UTF8 (Join-Path $wbDir "work-disk.log")
    $code = $LASTEXITCODE
    Remove-Item Env:WSL_UTF8
    [Console]::OutputEncoding = $was
    return $code
}
function Work-Disk([string]$what) { Run-Native $wsl @("-d", $Distro, "-u", "root", "--", "/usr/local/sbin/whaybench-work-disk", $what) }
# Not there on a first install: no error for the log, which would read as a failure.
$task = Get-ScheduledTask -TaskPath "\Whaybench\" -TaskName "MountWork" -ErrorAction SilentlyContinue
# A task from before it ran wsl.exe through a headless console (a window flashed at each start) is made again.
$hidden = $task -and ($task.Actions | Where-Object { $_.Execute -like "*\conhost.exe" })
if (-not ($task -and $hidden -and (Test-Path $vhd))) {
    # diskpart reads its script in the system code page: the folder's short (8.3) name keeps a profile folder named in
    # another script readable. Without short names an ASCII path is still fine.
    $short = $wbDir
    try { $short = (New-Object -ComObject Scripting.FileSystemObject).GetFolder($wbDir).ShortPath } catch {}
    if ($short -match '[^\x20-\x7e]') { $short = "" }
    $me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    # The elevated part, given whole on its command line (encoded): no file of it that another program could change
    # before it runs. It runs for this account only: another administrator's password would make the disk in that
    # account's profile and WSL.
    $elevated = @'
$ErrorActionPreference = "Stop"
if ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'ME') { exit 2 }
$vhd = 'VHD'
$short = 'SHORT'
if (-not (Test-Path $vhd)) {
    if (-not $short) { exit 3 }
    # Exactly 1 TiB, growing as it fills: the size is how whaybench-work-disk tells this disk from any other blank one.
    # diskpart, not New-VHD (that one needs Hyper-V's module); its command piped in, with no script file that another
    # program could change before diskpart runs it with these rights.
    $ErrorActionPreference = "Continue"
    "create vdisk file=`"$short\work.vhdx`" maximum=1048576 type=expandable" | & diskpart.exe | Out-Null
    if (-not (Test-Path $vhd)) { exit 4 }
    $ErrorActionPreference = "Stop"
}
$wsl = "$env:SystemRoot\System32\wsl.exe"
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 2)
# wsl.exe is a console program: run by the task it would flash a console window at each start; conhost --headless
# gives it a console without a window.
$action = New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\conhost.exe" -Argument "--headless `"$wsl`" --mount --vhd `"$vhd`" --bare"
$principal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Highest
Register-ScheduledTask -TaskPath "\Whaybench\" -TaskName "MountWork" -Action $action -Principal $principal -Settings $settings -Force | Out-Null
$ErrorActionPreference = "Continue"
& $wsl --mount --vhd $vhd --bare *> $null
exit 0
'@
    # Replaced as text: a path may hold $, which -replace reads as a group.
    $elevated = $elevated.Replace("'ME'", "'$me'").Replace("'VHD'", "'" + $vhd.Replace("'", "''") + "'").Replace("'SHORT'", "'" + $short.Replace("'", "''") + "'")
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($elevated))
    Say "Work disk: setting up (administrator rights requested once)"
    $code = -1
    try { $code = (Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList "-NoProfile -ExecutionPolicy Bypass -EncodedCommand $encoded").ExitCode } catch {}
    switch ($code) {
        0 {}
        -1 { Say "Work disk: not set up (administrator rights denied). ~/work: Linux system disk. Retry: run Setup again." }
        2 { Say "Work disk: not set up (administrator rights of another account; this account required). ~/work: Linux system disk." }
        3 { Say "Work disk: not set up (path not readable by diskpart, no short name: $wbDir). ~/work: Linux system disk." }
        default { Say "Work disk: not set up (code $code). Log: $wbDir\work-disk.log. ~/work: Linux system disk. Retry: run Setup again." }
    }
} else {
    # Set up before: attached by its task, without asking again.
    Run-Native "$env:SystemRoot\System32\schtasks.exe" @("/run", "/tn", "Whaybench\MountWork") | Out-Null
}
if (Test-Path $vhd) {
    # A start waits for the disk from now on, also when it could not be attached now.
    Work-Disk "expect" | Out-Null
    # The attach may take a moment: up to 20 seconds. Formats only the work disk (a blank 1 TiB one), and a disk with
    # its label is left as it is.
    $ready = $false
    for ($i = 0; $i -lt 20 -and -not $ready; $i++) { if ((Work-Disk "format") -eq 0) { $ready = $true } else { Start-Sleep -Seconds 1 } }
    if (-not $ready) { Say "Work disk: not formatted. Log: $wbDir\work-disk.log. Retry: run Setup again." }
    elseif ((Work-Disk "mount") -ne 0) { Say "Work disk: not mounted on ~/work. Log: $wbDir\work-disk.log. Next start: retried." }
}

# Shortcuts: wslg.exe starts it without a console window. It comes with WSL from the Store, beside its wsl.exe.
$wslg = Join-Path $env:ProgramFiles "WSL\wslg.exe"
if (-not (Test-Path $wslg)) { Fail "wslg.exe not found ($wslg). Update: 'wsl --update'. Then run Setup again." }
# The icon stays where the shortcuts can find it after this folder is gone.
$home_ = Join-Path $env:LOCALAPPDATA "Whaybench"
$ico = Join-Path $home_ "whaybench.ico"
New-Item -ItemType Directory -Force $home_ | Out-Null
$here = Join-Path $PSScriptRoot "whaybench.ico"
# Whaybench Setup already puts it there.
if ($here -ne $ico) {
    if (Test-Path $here) { Copy-Item $here $ico -Force }
    else { try { Invoke-WebRequest -UseBasicParsing -Uri $Icon -OutFile $ico } catch { $ico = $null } }
}
$shell = New-Object -ComObject WScript.Shell
$places = @([Environment]::GetFolderPath("Programs"), [Environment]::GetFolderPath("Desktop"))
foreach ($place in $places) {
    $link = $shell.CreateShortcut((Join-Path $place "Whaybench.lnk"))
    $link.TargetPath = $wslg
    $link.Arguments = "-d $Distro -- /usr/local/bin/whaybench-start"
    $link.Description = "Whaybench"
    if ($ico) { $link.IconLocation = "$ico,0" }
    $link.Save()
}
Say "Done. Start: Start menu or desktop. First start: sign in to Claude in the browser."
try { Stop-Transcript | Out-Null } catch {}
