# CANoxide

A small, free Windows tool to test, configure and debug CANopen devices
on a bench. It works with a PEAK PCAN-USB adapter, an ifm CANfox, a Kvaser
interface, an SLCAN device (CANable, CANUSB, USBtin, or a gateway over TCP),
or with no hardware at all through a built-in simulated node. The Kvaser
and SLCAN adapters are not tested on hardware yet.

- **GUI** (`CANoxide.exe`): one portable exe of about 9 MB, or a standard
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
  decimal and string; filter, pause, log to a file, save as CSV, PEAK
  `.trc` or Vector `.asc`. Play back a log (CANoxide CSV, candump, PEAK
  `.trc`, Vector `.asc`) as if it were the bus.
- **Decode your own messages.** Define signals in the bit grid, or import
  them from a DBC file.
- **Plot values.** Graph PDO signals, SDO-polled objects or raw frame bits
  over time, or show them on a **Dashboard** as large values, bars, dials
  and lamps with warning and alarm thresholds.
- **Configure a device.** Read and change its PDO configuration, set its
  node-ID and bit rate by LSS, save its configuration to a DCF, compare
  and restore it. A PDO check finds a mapping or a COB-ID that cannot work
  before it is written. Store its parameters (0x1010) or restore its
  defaults (0x1011).
- **Download a program** to a node by CiA 302-3 (objects 0x1F50, 0x1F51,
  0x1F56, 0x1F57): stop, optional clear, download, wait for the flash,
  start, wait for the boot-up, each step optional and reported.
- **Drive a CiA 402 axis.** Read its state and values, send the state
  machine commands, write the mode and the targets. The commands that can
  move the drive stay locked until you confirm that it can move safely.
- **Check a device against its EDS.** Read every entry of the EDS from the
  node and list what does not match: missing objects, wrong access, wrong
  size, constant values that differ. Read only; CSV and JUnit XML export.
- **Follow its emergencies.** A history of every EMCY on the bus with the
  CiA 301 code text and the error register bits, and the error history
  0x1003 of a node (read and clear).
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
compares the features of CANoxide 0.4 with those of CANopen Magic
Professional, taken from the vendor's public product pages in November
2026. Check the vendor's pages for the current and complete list.
CANoxide is younger and has had far less use on real networks.

| Feature | CANopen Magic Pro | CANoxide 0.4 |
|---|---|---|
| Price and licence | Commercial licence | Free, MIT, open source |
| EDS / DCF import, object dictionary browser | Yes | Yes |
| SDO read / write, expedited and segmented | Yes | Yes |
| SDO block transfer | Yes | Yes |
| SDO data to / from a file | Yes | Yes |
| Trace with CANopen decoding and filters | Yes | Yes |
| Several trace windows | Yes | Yes (up to 4 tabs) |
| Continuous logging to a file | Yes | Yes (CSV) |
| Log playback | Yes | Yes (CANoxide CSV, candump, PEAK `.trc` 1.0 to 2.1, Vector `.asc`) |
| Save a trace as PEAK `.trc` or Vector `.asc` | not stated | Yes (`.trc` 1.1, `.asc`) |
| DBC import (signals of non-CANopen messages) | not stated | Yes (no multiplexed signals) |
| Check of a PDO configuration before the write | not stated | Yes (length, EDS, COB-ID, transmission type) |
| PDO decoding from the device's real configuration | Yes | Yes |
| PDO configuration window (read, edit, write) | Yes | Yes |
| Graphical display of process data over time | Yes | Yes |
| Process data gauges and instrument panels | Yes | No |
| Network overview (state, identity, last emergency) | Yes | Yes |
| Emergency history | not stated | Yes (EMCY list with code texts, 0x1003 read and clear) |
| Check of a node against its EDS | not stated | Yes (read only, CSV and JUnit XML) |
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
| CAN interfaces | PEAK, Kvaser and others | PEAK PCAN-USB, ifm CANfox; Kvaser and SLCAN (not tested on hardware) |
| Portable single exe | No (installer) | Yes, plus an installer |
| Vendor support, long field history | Yes | No |

## User interface

The window has a menu bar, a connection bar (adapter, channel, bitrate,
listen-only, Connect, **Scan nodes**, bus state and load) and a status bar.
**Scan nodes** starts the node scan of the Network view and opens that
view; while the scan runs, the bar shows its progress and a **Cancel scan**
button. The views are dock tabs; open and close them from the **View**
menu:

- **Devices** and **EDS library**: the named devices and the EDS/DCF files.
- **Object dictionary**: the object tree of the active device, read and
  write by SDO, the manual SDO box, upload and download of large objects.
- **Network**: NMT for one node or all, SYNC, node scan, heartbeat, and an
  overview of the nodes heard on the bus. The **Parameters** group stores
  the parameters of the node (0x1010, "save") or restores its defaults
  (0x1011, "load", always asks first; the defaults are active after a
  reset node or a power cycle).
- **Emergencies**: every EMCY received, newest first: time, node and
  device name, code with its CiA 301 text, error register with its bits,
  manufacturer bytes. Node filter, **Clear**, **Export CSV...**. **Read
  error history (0x1003)** lists the error field of the node; **Clear error
  history** writes 0 to 0x1003:00. On the simulator, **Raise EMCY** and
  **Reset EMCY** make the simulated node send one.
- **Trace 1..4**: up to four trace tabs on one shared buffer, each with its
  own filter, display mode and pause; **Save trace...** writes CSV, PEAK
  `.trc` (version 1.1) or Vector `.asc`. With **Autoscroll** on,
  the view follows the newest frame; scroll up (wheel, Page Up, Home) and
  it stops; scroll back to the end (or End) and it follows again. With
  Autoscroll off, the visible rows stay in place while new frames arrive.
- **Messages**: one row per COB-ID with count, cycle time, DLC, the data
  bytes and the age. A byte that changed is tinted blue (value up) or red
  (value down) and fades out (fade time in Settings); **Mute noisy bytes**
  hides the tint of bytes that change on almost every frame. Select a row
  to see its bits (how often each bit toggles), the mapped PDO signals and
  their values; select bits and click **Add to Graph** to plot them as raw
  bits. Statistics reset for one row or for all. Signal analysis, see below.
- **Graph**: plot PDO signals (TPDOs and RPDOs of the device), SDO-polled
  values or raw frame bits; export as CSV. SDO polling runs only while you
  start it. The **Add signal** window can be moved, so that the plot stays
  visible.
- **PDO configuration**: read the PDOs of a node, edit, enable or disable
  them, save and load them from the library. Under the editor, the **PDO
  check** lists errors and warnings while you edit: more than 64 bits, an
  object that is not in the EDS, not mappable or of another length, the
  wrong access for the PDO direction, a COB-ID in a restricted range of
  CiA 301 or used by another PDO, a reserved transmission type. With an
  error, **Apply**
  asks first and **Write anyway** writes it.
- **Transmit list**: saved raw frames, sent once, periodically or on
  reception of a COB-ID.
- **LSS**: switch state, inquire, fastscan, configure node-ID and bit
  timing.
- **DCF save / restore**: save a node to a DCF, compare a DCF with a node,
  dry run and restore.
- **EDS check**: read each entry of the active device's EDS (on the
  simulator without a device: its built-in EDS) from the node and compare:
  ok, missing, write-only ok, access, size, value, abort, time-out. Options
  (skip DOMAIN, index range, stop after time-outs), a progress bar with
  Cancel, the summary line and the result table (**show ok entries**),
  **Export CSV...** and **Export JUnit XML...**. Nothing is written.
- **Program download** (File menu, and **Program download...** in the
  Network view): node, program number and file (size and SHA-256), the
  steps to run, then **Download program** (it asks first). The step list
  shows the progress in bytes, with Cancel; a failed step stops the
  sequence. See docs/CLI.md for the steps.
- **Drive (CiA 402)** (View menu, closed by default): polls 0x6041, 0x6061,
  0x6064, 0x606C, 0x6077 and 0x603F (an object that the drive does not
  have shows n/a). State chips, status bits, the commands that the state
  allows, the mode (0x6060). **Quick stop** and **Disable voltage** are
  always enabled while connected. **Enable operation** and the target
  writes (0x60FF, 0x607A, 0x6071) are in the **Motion (danger)** group,
  locked until "I have checked that the drive can move safely" is ticked;
  the tick is not saved and is cleared on disconnect and on a node change.
  In listen-only mode every command is disabled.
- **Dashboard** (View menu, closed by default): **Add signal...** adds a
  widget (Value, Bar, Dial or Lamp) with a title, min / max, decimals,
  warning and alarm thresholds and a size. It shares its signals with the
  Graph, so a signal is polled once. A value older than 5 poll periods (at
  least 2 s) is dimmed and shows its age.

**Log replay.** Choose **Log file (replay)** as the adapter and pick a
CANoxide trace CSV, a candump log (`(1700000000.123456) can0 181#0102`), a
PEAK `.trc` file (versions 1.0 to 2.1, with `$COLUMNS`) or a Vector `.asc`
file (hex or decimal base, absolute or relative time). The format is found
from the content of the file, not from its name. Records that are no
classic CAN frame (CAN FD frames, error frames, status and event lines)
are skipped; the status bar gives their count. Continuous logging stays
CSV. Connect loads the log (a log with bad lines asks first) and shows the
transport bar: play / pause, step, speed (0.25x to 16x or as fast as
possible), loop and a time scrubber. Every view works on the replayed
frames as on a live bus; nothing is sent. `canoxide-cli --adapter replay
--file <log>` does the same for scripts (see docs/CLI.md).

**Signal analysis** (Messages view, Graph, **Find signal** tab):

- **Define signal.** Drag over bits in the bit grid (also across byte
  borders; **LE** or **BE** sets the byte order of the selection) and click
  **Define signal...**: name, start bit, length, byte order, unsigned /
  signed / float32, factor, offset, unit and colour. User signals are kept
  per COB-ID in `user_signals.json`; they show as outlines and names in the
  grid, in the value list beside it (edit, delete), in the decoded text of
  the trace and as the **User signal** source of the Graph. They may overlap
  the PDO-mapped signals.
- **Import DBC...** (Messages toolbar and File menu): the messages and
  signals of a DBC file become user signals. A dialog shows what the file
  gives and what is skipped (multiplexed signals, float64, IDs that are not
  CAN IDs); **Add** keeps the existing signals, **Replace all signals of
  these IDs** first removes the user signals of the imported IDs. The DBC
  start bit is used as it is (Intel: the least significant bit; Motorola:
  the most significant bit in the DBC bit numbering). The file is read as
  UTF-8, else as Windows-1252.
- **Find similar bits.** Select one bit (or use a 1-bit user signal) and
  rank every bit of the trace buffer, or of the loaded log up to the replay
  cursor, by how often it equals the reference (match) or its inverse
  (inverted match). Bits that never toggled are left out; click a row to
  select that bit. The search runs in the background and can be cancelled.
- **Find signal** (View menu): start with every bit field of the chosen
  sizes (default: single bits and byte-aligned 8, 16 and 32 bit fields,
  optionally only some COB-IDs), then keep the fields whose newest value
  (live or at the replay cursor) passes each step: = != > < between, or
  increased / decreased / changed / unchanged since the last step. **Undo
  step**, **Reset**, and **Define signal...** for a candidate. The start
  set is capped at 200 000 fields (a note says so).
- **Sparklines** of the last 10 s (Settings: Messages: sparkline time)
  beside each user signal and each mapped PDO signal in the Messages
  detail.
- **History** of the selected COB-ID under the bit grid: newest frame
  first, time, Δt, data and one column per signal, with a filter (signal,
  = != > <, value) and **Export CSV...** of the filtered rows.
- **Graph > Export range**: a CSV of a time range (from / to, default the
  visible range) with a `time_s` column and one column per signal; a cell
  is empty when that signal has no sample at that time.

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
save/restore/compare, the EDS check (`check`, JUnit XML for CI), the
error history (`emcy history`, `emcy clear`), `store` and
`restore-defaults`, `dbc info`, the program download of CiA 302-3
(`program info`, `program download`), a CiA 402 drive (`drive status`,
`drive command`), and a `shell` that keeps one session open.
`program download` and `drive command enable` send nothing without `--yes`.
`pdo set` and `pdo map` run the PDO check first and stop on an error
unless `--force` is given. Output is
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
opens the channel. "Listen only" opens the PCAN channel (or the CANfox
net, with the silent flag of the Sontheim driver; the Kvaser channel in
the silent driver mode; the SLCAN channel with `L`) in listen-only
mode and disables every send control. The node scan sends one SDO request
at a time.

LSS and DCF restore change a device: a new node-ID or bit rate can take a
device off the bus, and a failed PDO change leaves that PDO disabled until
it is written again. A program download stops the program of the node, and
a wrong file can leave it without a working program. The Drive panel can
make a motor move. The tool asks before these actions (see the Safety
settings).

The PEAK adapter has had only light use on real hardware; the CANfox
adapter was tested on one unit only; the Kvaser and SLCAN adapters are
not tested on hardware at all. Try listen-only
mode on a bench bus before you rely on the tool on a live system.

## Data folder

All state of the GUI lives in one data folder: `settings.json`,
`devices.json`, `eds\` (copies of the added EDS/DCF files), the saved
PDO configurations, `layout.json` (the dock layout), `signals.json` (the
graph signals), `user_signals.json` (the user signals of the Messages
view), `txlist.json` (the transmit list) and `dashboard.json` (the
dashboard widgets; a data folder without it starts with an empty
dashboard).

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

## CANfox driver

The CANfox adapter needs the Sontheim / ifm CANfox driver installed (the
64-bit `SIECA164.DLL` of the Sontheim MT-API). It is looked for in the
path of the environment variable `CANOXIDE_SIECA_DLL`, then by name (the
Windows search), then in `Sontheim\MT_Api` under Program Files (x86) and
Program Files. Without it the app still works and the CANfox net list shows
the error. A `channel` is a net number: a CANfox is net 105. The adapter
never forces the bitrate of a net that another program (for example
CoDeSys) owns; when the driver does not answer, it stops after about 3 s
and asks you to close CoDeSys and restart the PC. Tested so far, on
one CANfox at 500 kbit/s: the net list, reception and one SDO read. It
is not known whether a CANfox in listen-only mode still
acknowledges frames.

## Kvaser driver

**Not tested on hardware: please report results** (an issue on GitHub).
The Kvaser adapter needs the Kvaser drivers with CANlib (`canlib32.dll`;
the 64-bit DLL has the same name), loaded at run time from `System32` or
from the exe folder. Without it the app still works and the channel list
shows "Kvaser driver not found: install the Kvaser drivers". A `channel`
is the CANlib channel number (0, 1, ...); the Kvaser virtual channels
work too. A channel is opened shared (not exclusive); listen only uses the
silent driver mode; the bus state comes from `canReadStatus`. As with the
CANfox, every driver call has a time-out of about 3 s. 20 and 800 kbit/s
have no predefined CANlib setting and are set from bit timing values
(16 and 10 time quanta per bit).

## SLCAN adapters

**Not tested on hardware: please report results** (an issue on GitHub).
"SLCAN (serial)" talks the Lawicel ASCII protocol on a COM port (CANable
or candleLight with slcan firmware, Lawicel CANUSB, USBtin; 115200 8N1,
which a USB device ignores). "SLCAN (TCP)" talks the same protocol to a
network gateway at `host:port` (for example a VSCOM NET-CAN in ASCII mode).
No driver DLL is needed. Opening sends `C`, `V`, `S<n>` and `O` (or `L`
for listen only), never a CAN frame; a device that does not answer `V`
is refused. Received time stamps are ignored (the PC time is used).
A lost connection or a removed device is reported once and the bus shows
"Connection lost"; connect again after you fix it.

What is not known without hardware:

- whether each device answers `V` (a device that does not is refused);
- what a device that does not know `L` does (the CANable firmware has no
  listen-only command: it is expected to ignore `L`, so the channel stays
  closed and nothing is received; it is never opened with `O` instead);
- the 800 kbit/s code `S7` (the CANable firmware uses `S7` for 750
  kbit/s);
- the COM port settings and time-outs on a real port, the COM port list,
  and how a removed USB device is seen;
- the time-outs of a real TCP gateway, and whether it closes the
  connection when the CAN side has a fault;
- the error flags of `F` on a real bus.

## Build

Rust stable (MSVC), from this folder:

```
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The exes are `target\release\CANoxide.exe` and
`target\release\canoxide-cli.exe`: single files with a static CRT. Both
carry the app icon (`assets\icon\canoxide.ico`, compiled by `build.rs`
with the `winresource` crate and the `rc.exe` of the Windows SDK);
`py -3 tools\check_exe_icon.py` checks the built exes. The installer is
built with Inno Setup 6, see [installer/README.md](installer/README.md).

A Rust exe contains the source paths of its dependencies, and so the name
of your user folder. For a build that you publish, replace it (PowerShell):

```
$env:RUSTFLAGS = "-C target-feature=+crt-static --remap-path-prefix=$env:USERPROFILE=C:\Users\user"
cargo build --release
```

## Limits (not built)

- CAN FD.
- Firmware download (CiA 302-3) as a guided function (a block download of
  a file to an object is possible).
- Gauges, error frame display, application profiles such as CiA 447.
- Adapters other than PEAK, the CANfox, Kvaser, SLCAN and the simulator
  (the `CanAdapter` trait allows more backends later).

## Licence

MIT. See [LICENSE](LICENSE). The embedded fonts (Inter, JetBrains Mono)
are under the SIL Open Font License, see `licenses\`. The icons are
Phosphor icons (MIT). The licences of the Rust crates and fonts in the
exes are in [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt).

No driver file of PEAK, Sontheim or Kvaser is part of CANoxide. Product and company
names in this file are trademarks of their owners and are used only to
describe compatibility.
