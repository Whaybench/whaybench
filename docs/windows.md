# Installing on Windows: when the setup stops

Whaybench runs on Linux. On Windows, **WhaybenchSetup.exe** installs WSL (Windows Subsystem for Linux, a Linux that
runs inside Windows) and everything else for you. Most problems come from two Windows settings that are turned off on
many computers. This page shows how to turn them on, one step at a time.

If the setup stops, it shows the reason in red and keeps a log at `%LOCALAPPDATA%\Whaybench\install.log`. After you fix
the problem, run **WhaybenchSetup.exe** again: it goes on where it stopped.

## 1. Check your Windows version

1. Press <kbd>Windows</kbd> + <kbd>R</kbd>, type `winver` and press <kbd>Enter</kbd>.
2. The window shows a version and an **OS Build** number.

You need **Windows 11**, or **Windows 10 version 21H2 or newer** (OS Build 19044 or higher). Windows Home is fine.
If yours is older, open **Settings → Windows Update**, install all updates, and check again.

## 2. Check that virtualization is on

WSL runs Linux in a small virtual machine, so the processor's virtualization feature must be on.

1. Press <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>Esc</kbd> to open **Task Manager**.
2. Open the **Performance** tab and click **CPU**.
3. Below the graph, find **Virtualization**.

- **Enabled**: good, go to step 3.
- **Disabled**: turn it on in the BIOS, as below.

### Turn on virtualization in the BIOS

The BIOS (or UEFI) is the computer's own settings screen, before Windows starts. Changing this one setting is safe.

1. Open **Settings → System → Recovery**. Next to **Advanced startup**, click **Restart now**.
   (Windows 10: **Settings → Update & Security → Recovery → Advanced startup → Restart now**.)
2. On the blue screen, choose **Troubleshoot → Advanced options → UEFI Firmware Settings → Restart**.
3. The BIOS opens. Look for a setting with one of these names, often under **Advanced**, **CPU Configuration** or
   **Security**:
   - Intel processors: **Intel Virtualization Technology**, **Intel VT-x** or **VT-x**
   - AMD processors: **SVM Mode**, **SVM** or **AMD-V**
4. Set it to **Enabled**.
5. Save and exit. On most computers this is <kbd>F10</kbd>, then **Yes**.

If **UEFI Firmware Settings** is not in the list, restart the computer and press the BIOS key over and over as soon as
it turns on. The key depends on the maker: often <kbd>F2</kbd> or <kbd>Del</kbd>; <kbd>F10</kbd> on HP; <kbd>F1</kbd>
on Lenovo ThinkPad. The maker's support site has the exact steps for your model.

## 3. Turn on the Windows features for WSL

The setup turns these on by itself. If it says **WSL could not be installed**, turn them on by hand:

1. Press <kbd>Windows</kbd> + <kbd>R</kbd>, type `optionalfeatures` and press <kbd>Enter</kbd>.
2. Check both:
   - **Virtual Machine Platform**
   - **Windows Subsystem for Linux**
3. Click **OK**, wait, and restart Windows when it asks.
4. Run **WhaybenchSetup.exe** again.

## Error codes

| You see | What it means | Fix |
|---|---|---|
| `0x80370102` | Virtualization is off, or Virtual Machine Platform is not on | Steps 2 and 3 |
| `0x8007019e` | Windows Subsystem for Linux is not on | Step 3 |
| `0x80070003` | Windows saves new apps on a drive other than `C:` | **Settings → System → Storage → Advanced storage settings → Where new content is saved**: set **New apps will save to** to `C:` (Windows 10: **Settings → System → Storage → Change where new content is saved**). Then run the setup again |
| `wslg.exe was not found` | WSL is old | Open **Command Prompt**, run `wsl --update`, then run the setup again |
| `Fedora could not be installed` | No internet, or virtualization is off | Check the internet connection, then step 2 |
| `WSL 2.4.4 or newer is needed` | WSL is old | Open **Command Prompt** as administrator, run `wsl --update`, then run the setup again |

## Other cases

- **Work or school computer**: the organisation may block WSL or BIOS changes. Ask your IT team to allow WSL 2.
- **Windows inside a virtual machine** (VMware, VirtualBox, Hyper-V): turn on nested virtualization for that virtual
  machine on the host, for example **Virtualize Intel VT-x/EPT or AMD-V/RVI** in VMware.
- **Very old processors** (before about 2010, such as Intel Core 2 Duo) cannot run WSL 2.

## The work disk

Your projects (`~/work` inside Whaybench) are kept on a disk of their own, `%LOCALAPPDATA%\Whaybench\work.vhdx`. It
grows as it fills, up to 1 TB, and stays when Whaybench or its Linux system is removed.

- The setup asks once for **administrator rights** to make and attach it. Without them Whaybench still runs, with
  `~/work` on its Linux system's own disk; run the setup again to make the disk later. It has to be your own Windows
  account with administrator rights: another account's password makes no disk for you.
- Each start attaches it through the scheduled task **Whaybench\MountWork**, without asking. If it does not come,
  the status line says **Work disk not attached**; why is in `~/.local/share/whaybench/logs/work-disk.log`
  inside Whaybench, and the setup's own steps in `%LOCALAPPDATA%\Whaybench\work-disk.log`.
- The first time, what was in `~/work` is copied onto the disk and the original is kept as `~/work.before-disk`.
  Files written to `~/work` while the disk was not attached are moved to `~/work.unmounted-<date>` when it comes
  back. Neither is deleted; remove them when you have checked them.

## Removing everything

**Settings → Apps → Whaybench → Uninstall** removes the program and its shortcuts, and asks whether to delete its
Linux system too. The work disk stays. To remove the rest by hand:

1. `wsl --unregister Whaybench` in **Command Prompt** (if you kept the Linux system): this deletes it and everything
   in it.
2. `wsl --shutdown`, then delete `%LOCALAPPDATA%\Whaybench\work.vhdx` (your projects) and the folder.
3. If the uninstaller said so, delete **Whaybench\MountWork** in **Task Scheduler**.

## Still stuck

Open an issue at [github.com/Whaybench/whaybench/issues](https://github.com/Whaybench/whaybench/issues) with the red
message and the log `%LOCALAPPDATA%\Whaybench\install.log`. The log can contain your Windows user name; remove it if
you prefer.

Microsoft's own guide: [Troubleshooting WSL](https://learn.microsoft.com/windows/wsl/troubleshooting).
