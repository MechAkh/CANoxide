# CANoxide

A small, free Windows tool to test, configure and debug CANopen devices
on a bench. It works with a PEAK PCAN-USB adapter, or with no hardware at
all through a built-in simulated node.

- **GUI** (`CANoxide.exe`): one portable exe of about 6 MB, or a standard
  Windows installer.
- **Command line** (`canoxide-cli.exe`): the same functions with JSON
  output, made for scripts and AI agents.

What it is for:

- **Talk to a device.** Load its EDS or DCF file, then read and write its
  objects by SDO (expedited, segmented and block transfer) from a tree of
  the object dictionary. A manual SDO box works without an EDS.
- **Watch the bus.** A live trace decodes NMT, SYNC, EMCY, heartbeat, SDO,
  PDO and LSS messages. With an EDS, SDO lines show object names and PDOs
  are split into their mapped signals. Switch the data view between hex,
  decimal and string; filter, pause, log to a file, save as CSV.
- **Plot values.** Graph PDO signals, SDO-polled objects or raw frame bits
  over time.
- **Configure a device.** Read and change its PDO configuration, set its
  node-ID and bit rate by LSS, save its configuration to a DCF, compare
  and restore it.
- **Control the network.** NMT start / stop / reset, node scan, network
  overview, SYNC, heartbeat set-up, and a transmit list of raw frames
  (manual, periodic or on reception).
- **Keep a library.** Named devices (node ID plus EDS) and your EDS files.
- **Try it without hardware.** The simulated node answers SDO, sends
  heartbeat and PDOs, reacts to NMT and LSS.

It is an independent open-source project, written in Rust with `egui`. It
is not affiliated with any commercial CANopen tool.

![CANoxide main window, dark theme](docs/screens/main_dark.png)

![CANoxide main window, light theme](docs/screens/main_light.png)

## Download

From the
[latest release](https://github.com/MechAkh/CANoxide/releases/latest),
Windows 10/11, 64 bit:

| File | Use |
|---|---|
| `CANoxide-x.y.z-setup.exe` | Installer with the usual setup wizard (Start menu entry, uninstaller, optional desktop shortcut and PATH entry for the command-line tool). |
| `CANoxide.exe` | Portable GUI: copy it anywhere and run it. |
| `canoxide-cli.exe` | Portable command-line tool. |

The files are not code signed, so Windows SmartScreen may warn on the
first start ("More info" then "Run anyway").

## Comparison with CANopen Magic Professional

CANopen Magic is the established commercial tool in this field. The table
compares the features of CANoxide 0.2 with those of CANopen Magic
Professional, taken from the vendor's public product pages in November
2026. Check the vendor's pages for the current and complete list.
CANoxide is younger and has had far less use on real networks.

| Feature | CANopen Magic Pro | CANoxide 0.2 |
|---|---|---|
| Price and licence | Commercial licence | Free, MIT, open source |
| EDS / DCF import, object dictionary browser | Yes | Yes |
| SDO read / write, expedited and segmented | Yes | Yes |
| SDO block transfer | Yes | Yes |
| SDO data to / from a file | Yes | Yes |
| Trace with CANopen decoding and filters | Yes | Yes |
| Several trace windows | Yes | Yes (up to 4 tabs) |
| Continuous logging to a file | Yes | Yes (CSV) |
| Log playback | Yes | No |
| PDO decoding from the device's real configuration | Yes | Yes |
| PDO configuration window (read, edit, write) | Yes | Yes |
| Graphical display of process data over time | Yes | Yes |
| Process data gauges and instrument panels | Yes | No |
| Network overview (state, identity, last emergency) | Yes | Yes |
| NMT, node scan, heartbeat set-up | Yes | Yes |
| Bus load display | Yes | Yes |
| Transmit list (manual, periodic, on reception) | Yes | Yes (no keyboard triggers) |
| LSS master, including Fastscan | Yes | Yes |
| DCF save and restore of a node configuration | Yes | Yes, with compare and dry run |
| Dockable, movable windows | Yes | Yes |
| Dark and light themes | not stated | Yes |
| Command-line tool with JSON output for scripts and AI agents | Command-line options | Yes (`canoxide-cli`) |
| Built-in simulated node | Ultimate edition (full node simulation) | One simple node |
| Scripting inside the tool | Ultimate edition (Python) | No (use the command-line tool) |
| CAN FD / CANopen FD | Yes | No |
| CiA 447 and other application profiles | Yes | No |
| Error frame display | Yes | No |
| CAN interfaces | PEAK, Kvaser and others | PEAK PCAN-USB only |
| Portable single exe | No (installer) | Yes, plus an installer |
| Vendor support, long field history | Yes | No |

## User interface

The window has a menu bar, a connection bar (adapter, channel, bitrate,
listen-only, Connect, bus state and load) and a status bar. The views are
dock tabs; open and close them from the **View** menu:

- **Devices** and **EDS library**: the named devices and the EDS/DCF files.
- **Object dictionary**: the object tree of the active device, read and
  write by SDO, the manual SDO box, upload and download of large objects.
- **Network**: NMT for one node or all, SYNC, node scan, heartbeat, and an
  overview of the nodes heard on the bus.
- **Trace 1..4**: up to four trace tabs on one shared buffer, each with its
  own filter, display mode and pause; save as CSV.
- **Graph**: plot PDO signals, SDO-polled values or raw frame bits; export
  as CSV. SDO polling runs only while you start it.
- **PDO configuration**: read the PDOs of a node, edit, enable or disable
  them, save and load them from the library.
- **Transmit list**: saved raw frames, sent once, periodically or on
  reception of a COB-ID.
- **LSS**: switch state, inquire, fastscan, configure node-ID and bit
  timing.
- **DCF save / restore**: save a node to a DCF, compare a DCF with a node,
  dry run and restore.

**Docking.** Drag a tab to move or split it, so that several views are
visible at once. The first start picks layout A (window narrower than
1600 px) or layout B (wider, Network on the right); **View > Reset
layout** restores it. The layout is saved in `layout.json`; a broken file
gives the default layout with a note.

**Themes.** Dark, Light, or System (follows Windows); the button at the
right of the menu bar cycles them. Both themes meet the WCAG AA contrast
for text.

**Settings** (menu **Settings**): appearance (theme, UI scale, font size,
row density, zebra stripes, reduced colour), connection defaults, SDO,
LSS, scan, trace, graph and safety options (for example the confirmation
before a write); the Data folder page shows where the data is kept.
Changes apply on **Save**; **Cancel** keeps the saved values.

## Command line

`canoxide-cli.exe` is a command-line CANopen master for scripts and AI
agents, on the same library: scan, node info, SDO read/write (block
transfer included), OD dump, NMT, SYNC, raw frames, a bounded bus monitor,
PDO read/enable/set/map, heartbeat, value watch, LSS, DCF
save/restore/compare, and a `shell` that keeps one session open. Output is
JSON (JSON Lines for `monitor`, `watch` and `shell`); `--text` gives
tables. The simulator is the default adapter, and `--listen-only` refuses
every command that transmits.

```
canoxide-cli help --json
canoxide-cli scan
canoxide-cli sdo read 0x1018:1
canoxide-cli --adapter pcan --listen-only monitor --duration 10
```

Exit codes: 0 ok, 1 device/protocol error, 2 usage error, 3 adapter error.
See [docs/CLI.md](docs/CLI.md) for the safety model and worked examples.

## Safety

Nothing is sent unless you press a button or give a command. Connect only
opens the channel. "Listen only" opens the PCAN channel in listen-only
mode and disables every send control. The node scan sends one SDO request
at a time.

LSS and DCF restore change a device: a new node-ID or bit rate can take a
device off the bus, and a failed PDO change leaves that PDO disabled until
it is written again. The tool asks before these actions (see the Safety
settings).

The PEAK adapter has had only light use on real hardware. Try listen-only
mode on a bench bus before you rely on the tool on a live system.

## Data folder

All state of the GUI lives in one data folder: `settings.json`,
`devices.json`, `eds\` (copies of the added EDS/DCF files), the saved
PDO configurations, `layout.json` (the dock layout), `signals.json` (the
graph signals) and `txlist.json` (the transmit list).

- **Portable:** `CANoxide_data\` next to the exe. A data folder of the old
  name (version 0.1) there is renamed to it on the first start.
- **Installed:** `%APPDATA%\CANoxide` (the installer puts a file
  `CANoxide.installed` next to the exe, which selects this folder).

If the folder is not writable, the app shows the error and saves nothing.
A broken `devices.json` is kept as `devices.json.bad`. The command-line
tool uses a data folder only when it gets `--data-dir`.

## PEAK driver

The PEAK adapter needs the PEAK driver with `PCANBasic.dll` (64 bit),
loaded at run time from `System32` or from the exe folder. Without it the
app still works (EDS browser, library, simulator) and the PEAK channel list
shows "driver not found". Channels: PCAN-USB 1..8.

## Build

Rust stable (MSVC), from this folder:

```
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The exes are `target\release\CANoxide.exe` and
`target\release\canoxide-cli.exe`: single files with a static CRT. The
installer is built with Inno Setup 6, see
[installer/README.md](installer/README.md).

## Limits (not built)

- CAN FD.
- Firmware download (CiA 302-3) as a guided function (a block download of
  a file to an object is possible).
- Log playback, gauges, error frame display, application profiles such as
  CiA 447.
- Adapters other than PEAK and the simulator (the `CanAdapter` trait
  allows more backends later).

## Licence

MIT. See [LICENSE](LICENSE). The embedded fonts (Inter, JetBrains Mono)
are under the SIL Open Font License, see `licenses\`.
