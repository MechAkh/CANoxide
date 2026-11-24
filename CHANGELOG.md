# Changelog

The notable changes of each release. The release pages on GitHub have the
download files and their SHA-256 checksums.

## Unreleased

### New

- **Emergencies view:** a history of the received EMCY messages with the
  error code as text, and the error history of a node (object 0x1003).
- **EDS check:** reads every object of a node and reports the differences
  from its EDS; `canoxide-cli check` gives the same as JSON or JUnit XML.
- **Store and restore parameters** (objects 0x1010 and 0x1011) in the
  Network view and in the command-line tool.
- `THIRD-PARTY-NOTICES.txt` with the licences of the Rust crates and fonts
  in the program.

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
