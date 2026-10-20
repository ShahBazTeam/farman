# Security Policy

## Supported versions

Security fixes are provided for the latest release and the `main` branch.
Older builds may be asked to upgrade before a report is investigated.

## Reporting a vulnerability

Please do not open a public issue for security problems.

Use GitHub's private vulnerability reporting:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**.
3. Include reproduction steps, affected version, and the impact you expect.

If GitHub reporting is unavailable, contact the maintainers through
<https://shahbazteam.ir>. Never include access tokens, passwords or private
shell output in a report. Use redacted paths and fixtures.

## What to include

- FARMAN version and commit if building from source.
- Windows version and display scale.
- The shell/profile and whether an IME or RTL layout was active.
- A minimal proof of concept or exact key sequence.
- Relevant log lines from `%LOCALAPPDATA%\FARMAN\logs` with secrets removed.
- Whether the issue requires local access, a malicious config, or a malicious
  terminal application.

## Relevant areas

Reports are especially useful for:

- ConPTY process/handle handling and shutdown.
- VT/OSC parsing, clipboard/OSC 52, or paste handling.
- Bidi/shaping paths that could corrupt selection, cursor mapping or copied
  logical text.
- Configuration parsing and path handling.
- Installer/uninstaller behaviour and registry writes.

## Disclosure

We will acknowledge a report as soon as practical, investigate it, and agree on
a disclosure timeline with the reporter. Please give us a reasonable chance to
ship a fix before publishing details.
