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

Every release is signed with minisign. The public key:

    RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia

`install.sh` and the built-in updater check the package against this key; with `minisign` installed, check a
downloaded package yourself:

    minisign -Vm whaybench-x86_64-linux.tar.gz -P RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia

## What Whaybench trusts

The agents in its tabs (Claude Code, Codex) run as you, with your files. Whaybench keeps its own data private (files
600, folders 700), keeps agents out of its settings and the view panel's browser data, asks before an agent add-on
runs for the first time, and marks text that one conversation sends to another.
