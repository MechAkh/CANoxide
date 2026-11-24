# Security

CANoxide is a bench tool. It has no network service and no update function;
it reads the files that you open (EDS, DCF, logs) and loads the CAN driver
DLL of the adapter that you choose.

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
