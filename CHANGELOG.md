# Changelog

The notable changes of each release. The release pages on GitHub have the
download files and their SHA-256 checksums.

## Unreleased

### New

- **Extended (29-bit) IDs in every view:** the identity of a frame is its
  number and its format, so a standard and an extended frame with the same
  number are two rows in Messages, two Trace rows in fixed mode, and two
  signals for the Graph, user signals and Find signal. A standard ID shows
  as 3 hex digits, an extended ID as 8 (Trace, Messages, Graph, transmit
  list, CLI output). An ID field reads hex: above 7FF, 8 digits or a
  trailing `x` (`123x`) is extended; more than 29 bits is an error.
- **Trace filter:** the COB-ID range takes extended IDs (all standard IDs
  sort before all extended IDs), and a new **Frame format** filter shows
  all frames, standard only or extended only.
- **Transmit list:** extended entries from the ID text or the Ext box;
  **On receive** waits for the typed ID in its own format.
- **Simulator: extended-ID demo frames** (Settings, Connection, off by
  default): the simulator also sends `0CF00400` every 50 ms, `18FEF100`
  every 100 ms and `18FEEE00` every second, with changing data.
- **CLI:** `send` and `monitor --id` read the same ID forms; `send 0x800`
  now sends an extended frame (before: an error). `monitor --id` keeps the
  two formats apart.
- **J1939 decoder (receive only):** 29-bit frames are decoded as J1939 by
  default (Settings, Connection: **Extended IDs are** J1939 or Raw frames).
  The Trace shows type `J1939`, the source address as the node, and a text
  with the PGN, its name for the common groups, the priority and the
  destination; the transport protocol (TP.CM / TP.DT, BAM and RTS/CTS) is
  followed per source and destination, and the last packet shows the
  reassembled message. Address claim NAME, Request, Acknowledgement and
  DM1 / DM2 faults are decoded. New Trace filters **PGN** and **Source
  address**; Messages shows `PGN n NAME  SA n` per ID. Nothing is sent:
  no address claim, no request, no CTS.
- **J1939 tab** (View menu): the parameter groups by PGN and source
  address (with a hex dump and the DM1 / DM2 faults of the selected row),
  the addresses with their claimed NAME, the active faults (a fault that
  is no longer reported stays dimmed until Clear), the transport counters
  and `Export CSV...`.
- **User and Graph signals by PGN:** Define signal has **Match the PGN
  (any source address)** with an optional source address; a DBC file with
  J1939 messages offers **Match by PGN (J1939)** on import (a message with
  source address 254 matches any address).
- **Simulator:** the extended-ID demo is a small J1939 node: EEC1, CCVS and
  ET1 from address 0, ET1 from address 33, an address claim at the start
  and a DM1 with two faults by BAM every second.
- **CLI:** `monitor` adds a `j1939` field (`pgn`, `name`, `priority`, `sa`,
  `da`) to each J1939 frame and has the class `j1939`; `--ext-raw` decodes
  29-bit frames as raw frames. New `j1939 decode-id <id>` (no bus).
- **Update notice:** at start (at most once a day) and on **Help > Check
  for updates...**, the app asks GitHub for the latest release: one HTTPS
  GET of `https://api.github.com/repos/MechAkh/CANoxide/releases/latest`
  with the headers `User-Agent: CANoxide/<version>` and
  `Accept: application/vnd.github+json`, through the system proxy settings.
  Nothing else is sent (no ID, no settings, no machine or user name), no
  cookie is kept and no redirect is followed. A newer release shows an
  **Update x.y.z** button in the menu bar; it opens a window with the two
  versions, the release name and date, **Open the release page** (in your
  browser), **Skip this version** and **Close**. Nothing is downloaded or
  run. Turn the check at start off in Settings, Updates: **Check for
  updates at start**. A failed check at start says nothing; the CLI never
  checks.

### Changed

- Buffer sizes: the Graph buffer has the same choices as the Trace buffer (10 000 to 1 000 000); the default of both is 500 000 (before: Trace 100 000 frames, Graph 20 000 points per signal). A stored setting keeps its value.

### Fixed

- Graph: with Follow off, the points older than the time window were not drawn after a drag or zoom back; they are drawn as long as they are in the buffer (Settings: Buffer size).

- The transmit list **On receive** trigger lost the extended format of its
  ID when an entry was edited, and took a standard ID above 7FF as
  extended only by its value.
- Find signal with a typed ID searched the standard and the extended frame
  of that number together.
- A raw Graph signal of an extended frame was named and listed like the
  standard frame of the same number.
- `canoxide-cli send` printed an extended ID with 3 digits; the bus status
  line of a failed send did the same.
- Old `txlist.json` files without the `extended` fields load as standard
  entries.

## 0.5.2 (2026-11-26)

### Fixed

- **Toolbar rows at some panel widths:** a combo box, a status pill or the
  Trace counters that do not fit in the rest of a toolbar row go to the next
  row. Before, at some widths the Trace counters were drawn over the Clear,
  Save, New tab and Filters buttons, and a combo box (for example `Type` and
  `Transfer` in the Manual SDO row) ran past the end of its row, under the
  next button or out of the panel. A wide panel looks as before.
- **Connection bar and status bar at a small window width:** the
  connection state and the bus load keep their place at the right end of
  the connection bar, and the adapter controls go on to a second row when
  they do not fit; before, the state and the Load meter were drawn over
  Scan nodes. In the status bar the message is cut with an ellipsis (the
  full text is shown on hover) before the Trace fill and the data folder;
  before, a long message was drawn over them.

## 0.5.1 (2026-11-25)

### Fixed

- **Dashboard widget layout:** each widget card is laid out top down in
  its own rectangle. In 0.5.0 the value text showed one character per line,
  the title and the Edit / Remove buttons were outside the card frame, a
  bar was far below its title and the cards had different heights.

### New

- **Dashboard canvas:** drag the title row of a widget to move it, the
  corner grip to resize it (120 x 90 to 800 x 600 points). Place and size
  snap to a 10-point grid, shown during the drag, and are saved in
  `dashboard.json`. The last moved, resized or added card is on top. The
  value text, the bar, the dial and the lamp scale with the card. **Arrange**
  places the widgets in rows that fit the visible width. A new widget goes
  to the first free place. A `dashboard.json` of 0.5.0 loads with its
  widgets arranged.
- **Messages view, Shift + click:** extends the bit selection from its
  first bit to the clicked bit, or to whole bytes on a `Byte n` label. The
  user signals with bits in the selection are marked in the value list.
- **Messages view, bit view position:** bottom (as before), left, right or
  top of the table, saved in the settings.
- **Messages view, to Graph and to Dashboard:** two buttons on each user
  signal and mapped PDO signal of the bit view (and on a Find signal
  candidate that is a defined signal). A signal that the Graph already
  shows is not added twice.

## 0.5.0 (2026-11-25)

### New

- **Emergencies view:** a history of the received EMCY messages with the
  error code as text, and the error history of a node (object 0x1003).
- **EDS check:** reads every object of a node and reports the differences
  from its EDS; `canoxide-cli check` gives the same as JSON or JUnit XML.
- **Store and restore parameters** (objects 0x1010 and 0x1011) in the
  Network view and in the command-line tool.
- `THIRD-PARTY-NOTICES.txt` with the licences of the Rust crates and fonts
  in the program.
- **PEAK and Vector trace files:** log replay reads PEAK `.trc` files
  (versions 1.0 to 2.1, `$COLUMNS` included) and Vector `.asc` files (hex
  or decimal base, absolute or relative time). Records that are no classic
  CAN frame (CAN FD, error frames, events) are skipped and counted. The
  format is found from the content. **Save trace...** writes CSV, `.trc`
  version 1.1 or `.asc`.
- **Import DBC:** the messages and signals of a DBC file become user
  signals (Messages view and File menu). Add them, or replace all signals
  of the imported IDs. Multiplexed signals are skipped and counted.
  `canoxide-cli dbc info` shows what an import gives.
- **PDO check:** the PDO editor lists errors and warnings while you edit
  (length over 64 bits, an object that is not in the EDS or not mappable,
  a reserved or duplicate COB-ID, a reserved transmission type). With an
  error, Apply asks before it writes. `canoxide-cli pdo set` and `pdo map`
  stop with exit code 2 (kind `pdo_check`) unless `--force` is given.
- **Program download (CiA 302-3):** a wizard (File menu and Network view)
  stops the program (0x1F51), optionally clears it, downloads a file to
  0x1F50, waits for the flash (0x1F57), checks its error code, starts the
  program, waits for the boot-up and reads the program identification
  (0x1F56) before and after. Each step is optional and reported, with the
  progress in bytes and Cancel; a failed step stops the sequence.
  `canoxide-cli program info` and `program download` (which needs `--yes`)
  do the same. The simulator has the program objects.
- **Drive (CiA 402) panel:** polls the statusword and the actual values,
  shows the state and the status bits, sends the state machine commands
  that the state allows and writes the mode. Enable operation and the
  target writes are locked until the drive is confirmed safe to move;
  Quick stop and Disable voltage are always enabled while connected.
  `canoxide-cli drive status` and `drive command` (`enable` needs `--yes`).
  The simulator has a CiA 402 axis.
- **Dashboard:** large values, bars, dials and lamps of the Graph signals,
  with warning and alarm thresholds and dimmed stale values. Saved in the
  data folder.
- **SLCAN adapters** (Lawicel ASCII protocol: CANable / candleLight slcan
  firmware, Lawicel CANUSB, USBtin, network gateways). GUI: "SLCAN
  (serial)" with the COM port list and "SLCAN (TCP)" with a host:port
  field. CLI: `--adapter slcan --channel COM5` or `--channel host:port`.
  Opening sends no CAN frame; a lost connection is reported once. Not
  tested on hardware.
- **Kvaser adapter** through CANlib (`canlib32.dll`, loaded at run time).
  GUI: a "Kvaser" entry with the channel list; virtual channels work.
  CLI: `--adapter kvaser --channel <number>`. Every driver call has a
  time-out. Not tested on hardware.

### Changed

- The **Save CSV...** button of the trace is now **Save trace...**.

## 0.4.0 (2026-11-20)

### New

- **ifm CANfox adapter** through the Sontheim MT-API. The driver's
  `SIECA164.DLL` is loaded at run time. GUI: a "CANfox" entry in the adapter
  picker with the net list. CLI: `--adapter canfox`, `--channel <net>`.
- Every CANfox driver call has a time-out, so a driver that hangs cannot
  freeze the program.
- Opening a CANfox net sends nothing and never forces the bitrate of a net
  that another program owns.

### Fixed

- A receive error that lasts is reported once, not on every poll.

## 0.3.3 (2026-11-20)

### New

- Network: "Add as device" on each scan result.
- Messages, bit view: a bit that changes is filled and then fades, and so
  is the Hex cell of its byte.
- Help > Copy display diagnostics.

### Fixed

- Cut-off letters on PCs with two graphics chips: the window is drawn on
  the low-power graphics chip; OpenGL remains as the fallback.
- Object dictionary: the Read / Write column always shows both buttons.

## 0.3.2 (2026-11-19)

- Messages: Hex / Decimal switch for the data bytes; a labelled Close
  button on the bit view.

## 0.3.1 (2026-11-19)

- Object dictionary: adjustable columns. Bit view: a close button.
- Graph: Follow goes off on a click, drag or scroll in the plot; zoom
  buttons.
- Trace: one tooltip on the Decoded cell. Messages: a Decoded column.
- The window restores its maximised state, size and position.

## 0.3.0 (2026-11-19)

- Messages view with the data-change display, Scan nodes button, RPDO
  signals in the Graph.
- Log replay of CANoxide CSV and candump logs, in the GUI and the CLI.
- Signal analysis: user signals, similar bits, Find signal, history.
- App icon and Windows installer.
