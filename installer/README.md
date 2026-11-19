# CANoxide Windows installer

`CANoxide.iss` is an [Inno Setup 6](https://jrsoftware.org/isinfo.php)
script (tested with 6.7.3). It builds a standard Windows setup program with
the usual wizard: welcome, licence, install mode, destination folder, Start
menu group, tasks, ready, progress and finish (with "Launch CANoxide").

## Build

Requirements: Rust (cargo) and Inno Setup 6.

```powershell
# Build the release exes and the setup program in one step:
powershell -NoProfile -ExecutionPolicy Bypass -File installer\build.ps1 -Build

# Or, when target\release already holds CANoxide.exe and canoxide-cli.exe:
powershell -NoProfile -ExecutionPolicy Bypass -File installer\build.ps1
```

`build.ps1` reads the version from `Cargo.toml`, checks that both release
exes exist (it runs cargo only with `-Build`), finds `ISCC.exe` (the
per-user Inno Setup install in `%LOCALAPPDATA%\Programs\Inno Setup 6`, then
`Program Files (x86)\Inno Setup 6`, then `PATH`) and compiles the script. It
prints the path, size and SHA-256 of the result:

```
target\installer\CANoxide-<version>-setup.exe
```

Use `-ExeDir <folder>` to package exes from another folder. To call the
compiler directly:

```powershell
ISCC.exe /DAppVersion=0.3.1 [/DExeDir=..\target\release] installer\CANoxide.iss
```

`AppVersion` is required; `ExeDir` is relative to the script and defaults to
`..\target\release`. A `licenses\` folder at the repository root (third-party
licence texts) is included when it exists at build time.

## What it installs

| Item | Where |
|---|---|
| `CANoxide.exe`, `canoxide-cli.exe`, `LICENSE`, `README.md`, `docs\CLI.md`, `licenses\` | the install folder |
| `CANoxide.installed` (marker file) | the install folder |
| Start menu: CANoxide, Uninstall CANoxide | the Start menu group |
| Desktop shortcut | only with the `desktopicon` task (off by default) |
| Install folder on `PATH` | only with the `addtopath` task (off by default) |

Install mode:

- **Just me** (default, no admin rights): `%LOCALAPPDATA%\Programs\CANoxide`,
  the user `PATH`, the uninstall entry under `HKCU`.
- **All users** (asks for admin rights): `C:\Program Files\CANoxide`, the
  system `PATH`, the uninstall entry under `HKLM`.

The `addtopath` task adds the install folder once (a second install does not
add it again); the uninstaller removes it. Running programs are told about
the change; a console that is already open keeps its old `PATH`.

A later version installs over the earlier one in place (fixed `AppId`). A
running CANoxide is closed before its files are replaced.

## Where the data goes

The marker file `CANoxide.installed` next to the exe tells CANoxide to keep
its settings and device library in `%APPDATA%\CANoxide` instead of the
portable `CANoxide_data` folder next to the exe.

The uninstaller removes the program files and the `PATH` entry. It does not
delete `%APPDATA%\CANoxide` on its own: an interactive uninstall asks "Also
delete your settings and device library?" (default No); a silent uninstall
always keeps the data.

## Silent install

```powershell
# Per user, into a chosen folder, with canoxide-cli on PATH:
CANoxide-0.3.1-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER /NORESTART `
    /DIR="D:\Tools\CANoxide" /TASKS="addtopath" /LOG="install.log"

# All users (run from an elevated prompt):
CANoxide-0.3.1-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /ALLUSERS /TASKS="addtopath,desktopicon"

# Silent uninstall (keeps %APPDATA%\CANoxide):
"<install folder>\unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

| Option | Meaning |
|---|---|
| `/SILENT`, `/VERYSILENT` | no wizard; `/VERYSILENT` also hides the progress window |
| `/SUPPRESSMSGBOXES` | answer message boxes with their default |
| `/CURRENTUSER`, `/ALLUSERS` | install mode without the dialog |
| `/DIR="..."` | install folder |
| `/GROUP="..."`, `/NOICONS` | Start menu group, or none |
| `/TASKS="..."` | comma list of `desktopicon`, `addtopath`; unlisted tasks are off |
| `/LOG="..."` | write a setup log |
| `/NORESTART` | never restart Windows |

The silent install does not start CANoxide.

## Code signing

The setup exe and the program exes are **not code signed**. Windows
SmartScreen warns on the first run of a downloaded setup ("Windows protected
your PC"): choose "More info", then "Run anyway". Check the SHA-256 that
`build.ps1` prints against the published value first.

## Test

`test_install.ps1` runs a silent per-user install into a new folder under
`target\installtest` (with `addtopath`), checks the files, the `HKCU`
uninstall entry, the user `PATH` and `canoxide-cli version`, installs a
second time, uninstalls silently and checks that everything is gone and that
the user `PATH` is byte-identical to its value before the test. It never
starts the GUI and opens no window.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File installer\build.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File installer\test_install.ps1
```
