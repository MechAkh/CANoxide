# Security

CANoxide is a bench tool. It has no network service and does not update
itself; it reads the files that you open (EDS, DCF, logs) and loads the CAN
driver DLL of the adapter that you choose.

Its only network traffic is the update check of the GUI: one HTTPS GET of
`https://api.github.com/repos/MechAkh/CANoxide/releases/latest` with the
headers `User-Agent: CANoxide/<version>` and
`Accept: application/vnd.github+json`, at start at most once a day (Settings,
Updates: **Check for updates at start**) and on **Help > Check for
updates...**. It uses the system proxy settings, accepts no certificate
fault, follows no redirect and sends no cookie and no logon. It reads the
version, the name, the date and the page address of the release; it
downloads and runs nothing, and opens the release page in the browser only
on a click. The command-line tool never makes this request.

## Reporting a problem

Please report a security problem through
[a private security advisory](https://github.com/MechAkh/CANoxide/security/advisories/new)
and not in a public issue. Only the latest release gets fixes.

## Checking a download

The release files are not code signed. Each release page lists the SHA-256
checksum of its files. In PowerShell:

```
Get-FileHash .\CANoxide.exe -Algorithm SHA256
```
