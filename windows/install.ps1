# Installs Whaybench on Windows: WSL if it is missing, an Ubuntu distribution of its own named Whaybench, everything
# inside it (setup-wsl.sh), and shortcuts that start it without a console window. Run it again to finish an install that
# stopped (after the restart WSL may ask for, for one): every step skips what is already there.
#   powershell -ExecutionPolicy Bypass -File install.ps1
param(
    [string]$Distro = "Whaybench",
    [string]$Ubuntu = "Ubuntu-26.04",
    [string]$Setup = "https://github.com/Whaybench/whaybench/releases/latest/download/setup-wsl.sh",
    [string]$Icon = "https://github.com/Whaybench/whaybench/releases/latest/download/whaybench.ico"
)
$ErrorActionPreference = "Stop"
function Say($text) { Write-Host "Whaybench: $text" }
function Fail($text) { Write-Host "Whaybench: $text" -ForegroundColor Red; exit 1 }
# wsl.exe writes UTF-16; read it as such.
[Console]::OutputEncoding = [System.Text.Encoding]::Unicode
$wsl = "$env:SystemRoot\System32\wsl.exe"

# WSLg, which shows Linux windows, needs Windows 11 or Windows 10 21H2 (build 19044) with WSL from the Store.
$build = [int](Get-CimInstance Win32_OperatingSystem).BuildNumber
if ($build -lt 19044) { Fail "Windows 10 21H2 or newer is needed (this is build $build). Update Windows first." }

# WSL itself. Without it, wsl.exe --status fails or says it is not installed.
& $wsl --status *> $null
if ($LASTEXITCODE -ne 0) {
    Say "installing WSL (this needs administrator rights)"
    & $wsl --install --no-distribution
    if ($LASTEXITCODE -ne 0) {
        Fail "WSL could not be installed. If the message speaks of virtualization, turn it on in the computer's BIOS (often called Intel VT-x, AMD-V or SVM) and run this again."
    }
    # A first WSL install usually needs a restart; this runs again by itself once Windows is back.
    $again = "powershell -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    Set-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce" -Name "WhaybenchSetup" -Value $again
    Say "restart Windows to finish; the installation continues by itself after the restart"
    exit 3010
}
& $wsl --update *> $null

# The distribution of its own: the person's other Linux distributions are not touched.
$have = (& $wsl --list --quiet) -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ -eq $Distro }
if (-not $have) {
    Say "installing $Ubuntu as $Distro (a few minutes)"
    & $wsl --install $Ubuntu --name $Distro --no-launch
    if ($LASTEXITCODE -ne 0) { Fail "Ubuntu could not be installed. Check the internet connection and run this again." }
}

# Inside: an account named after the Windows one, the programs, the input method for the Windows display language.
$user = ($env:USERNAME.ToLower() -replace '[^a-z0-9_-]', '')
if ($user -notmatch '^[a-z_]') { $user = "whaybench" }
$lang = (Get-UICulture).TwoLetterISOLanguageName
Say "setting up $Distro for $user (packages, Firefox, Claude Code, Whaybench; this takes a while)"
$script = Join-Path $env:TEMP "whaybench-setup-wsl.sh"
if (Test-Path (Join-Path $PSScriptRoot "setup-wsl.sh")) { Copy-Item (Join-Path $PSScriptRoot "setup-wsl.sh") $script -Force }
else { Invoke-WebRequest -UseBasicParsing -Uri $Setup -OutFile $script }
# Line ends as Linux reads them. The distribution reads the file where it is, from its folder (--cd takes a Windows
# path): text piped from PowerShell gets a Windows line end added, which the shell would run as a command, and
# Windows PowerShell breaks quotes inside arguments to wsl.exe.
[IO.File]::WriteAllText($script, ([IO.File]::ReadAllText($script) -replace "`r`n", "`n"))
& $wsl -d $Distro -u root --cd (Split-Path $script) -- sh (Split-Path -Leaf $script) $user $lang
if ($LASTEXITCODE -ne 0) { Fail "the setup inside $Distro stopped. Run this again to continue." }
# systemd and the default account apply from the next start of the distribution.
& $wsl --terminate $Distro *> $null

# Shortcuts: wslg.exe starts it without a console window. It comes with WSL from the Store, beside its wsl.exe.
$wslg = Join-Path $env:ProgramFiles "WSL\wslg.exe"
if (-not (Test-Path $wslg)) { Fail "wslg.exe was not found in $wslg. Run 'wsl --update' and run this again." }
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
Say "done. Start Whaybench from the Start menu or the desktop; the first time, sign in to Claude in the browser that opens."
