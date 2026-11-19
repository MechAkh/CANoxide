# CANoxide

A small, portable Windows tool to test and debug CANopen devices on a
bench. The GUI is one exe of about 6 MB, with no installer, and it works
with a PEAK PCAN-USB adapter. A second exe is a command-line tool for
scripts and AI agents.

What it is for:

- **Talk to a device.** Load its EDS or DCF file, then read and write its
  objects by SDO (expedited and segmented) from a tree of the object
  dictionary. A manual SDO box works without an EDS.
- **Watch the bus.** A live trace decodes NMT, SYNC, EMCY, heartbeat, SDO
  and PDO messages. With an EDS, SDO lines show object names and PDOs are
  split into their mapped signals. Switch the data view between hex,
  decimal and string; filter, pause, and save as CSV.
- **Control the network.** NMT start / stop / reset, node scan, SYNC, and a
  raw frame sender (one shot or cyclic).
- **Keep a library.** Named devices (node ID plus EDS) and your EDS files
  are stored in a folder next to the exe.
- **Try it without hardware.** A built-in simulated CANopen node answers
  SDO, sends heartbeat and PDOs, and reacts to NMT.

It is an independent open-source project, written in Rust with `egui`. It
is not affiliated with any commercial CANopen tool.

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
- **Transmit list**: saved raw frames, sent once or periodically.
- **LSS**: switch state, inquire, fastscan, configure node-ID and bit
  timing.
- **DCF save / restore**: save a node to a DCF, compare a DCF with a node,
  dry run and restore.

**Docking.** Drag a tab to move or split it. The first start picks layout
A (window narrower than 1600 px) or layout B (wider, Network on the
right); **View > Reset layout** restores it. The layout is saved in
`layout.json`; a broken file gives the default layout with a note.

**Themes.** Dark, Light, or System (follows Windows); the button at the
right of the menu bar cycles them. Both themes meet the WCAG AA contrast
for text.

**Settings** (menu **Settings**): appearance (theme, UI scale, font size,
row density, zebra stripes, reduced colour), connection defaults, SDO,
LSS, scan, trace, graph and safety options (for example the confirmation
before a write); the Data folder page shows where the data is kept. Changes apply on **Save**;
**Cancel** keeps the saved values.

## Download

Get `CANoxide.exe` from the
[latest release](https://github.com/MechAkh/CANoxide/releases/latest)
and run it. Windows 10/11, 64 bit.

## Build

Rust stable (MSVC), from this folder:

```
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The exe is `target\release\CANoxide.exe`: one file, static CRT, no
installer, no console window. Copy it anywhere.

## Command line

`canoxide-cli.exe` (built next to the GUI exe) is a command-line CANopen
master for scripts and AI agents, on the same library: scan, node info,
SDO read/write (block transfer included), OD dump, NMT, SYNC, raw frames,
a bounded bus monitor, PDO read/enable/set/map, heartbeat, value watch,
LSS, DCF save/restore/compare, and a `shell` that keeps one session open.
Output is JSON (JSON Lines for `monitor`, `watch` and `shell`); `--text`
gives tables. The simulator is the default adapter, and `--listen-only`
refuses every command that transmits.

```
canoxide-cli help --json
canoxide-cli scan
canoxide-cli sdo read 0x1018:1
canoxide-cli --adapter pcan --listen-only monitor --duration 10
```

Exit codes: 0 ok, 1 device/protocol error, 2 usage error, 3 adapter error.
See [docs/CLI.md](docs/CLI.md) for the safety model and worked examples.

## Data folder

All state of the GUI lives in one data folder: `settings.json`,
`devices.json`, `eds\` (copies of the added EDS/DCF files), the saved
PDO configurations, `layout.json` (the dock layout), `signals.json` (the
graph signals) and `txlist.json` (the transmit list).

- **Portable (default):** `CANoxide_data\` next to the exe. A data folder
  of the old name (version 0.1) there is renamed to it on the first start.
- **Installed:** when a file `CANoxide.installed` is next to the exe, the
  folder is `%APPDATA%\CANoxide`.

If the folder is not writable, the app
shows the error and saves nothing. A broken `devices.json` is kept as
`devices.json.bad`. The command-line tool uses a data folder only when it
gets `--data-dir`.

## PEAK driver

The PEAK adapter needs the PEAK driver with `PCANBasic.dll` (64 bit),
loaded at run time from `System32` or from the exe folder. Without it the
app still works (EDS browser, library, simulator) and the PEAK channel list
shows "driver not found". Channels: PCAN-USB 1..8.

## Safety

Nothing is sent unless you press a button. Connect only opens the channel.
"Listen only" opens the PCAN channel in listen-only mode and disables every
send control. The node scan sends one SDO request at a time.

## Limits (not built)

- Firmware download (CiA 302-3).
- CAN FD.
- Adapters other than PEAK and the simulator (the `CanAdapter` trait allows
  a CANfox backend later).
- No installer.

## Licence

MIT. See [LICENSE](LICENSE).
