# Security

## Reporting a vulnerability

Report it privately through GitHub: the repository's **Security** tab → **Report a vulnerability**. Do not open a
public issue for it. Say what an attacker can do, the version (`whaybench --version`) and the steps that show it.

Whaybench is made by one person, so this is best effort: an answer usually comes within a week. A fix goes into the
next release; the release notes name the problem once the fixed version is out.

## Supported versions

Only the latest release gets fixes. Whaybench downloads a new release by itself and installs it at the next start
(Whaybench → Restart); an older version is fixed by updating.

## Checking a release

From 0.6.1 on, `install.sh`, the package (`whaybench-x86_64-linux.tar.gz`) and `setup-wsl.sh` of each release are signed
with minisign, and `WhaybenchSetup.exe` from 0.9.1 on. The public key:

    RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia

- The built-in updater checks `install.sh` and the package against this key, always.
- `install.sh` checks the package against it when `minisign` is installed; without it, only the checksum, and it says
  so. Run straight from the network (`curl ... | sh`), `install.sh` itself is not checked.
- On Windows, the setup inside WSL installs `minisign` and checks `install.sh` before it runs it.

With `minisign` installed, check one of these files yourself (its `.minisig` beside it):

    minisign -Vm whaybench-x86_64-linux.tar.gz -P RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia

## What Whaybench trusts

Whaybench runs on your computer within the boundary of your user account, and so do the agents in its tabs. Each agent
follows its own permission settings; Whaybench starts Codex in its `workspace-write` sandbox.

Whaybench keeps its data readable by your account only, asks before an agent add-on that did not come with it, or was
changed, first runs, and marks text that one conversation sends to another.

Pages are shown by WebKitGTK and Firefox. Whaybench does not check pages, files or links for harmful content.
