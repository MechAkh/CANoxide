# Changelog

The notable changes of each release. The release pages on GitHub have the
download files and their SHA-256 checksums.

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
