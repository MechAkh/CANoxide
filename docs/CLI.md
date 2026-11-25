# canoxide-cli: guide for scripts and AI agents

`canoxide-cli.exe` is the command-line twin of the CANoxide GUI. It
uses the same library: the same SDO client, PDO planner, DCF and LSS code,
and the same simulator. It is built for scripts and for AI agents: every
result is one JSON object, every error has a kind and an exit code, and the
full command table is machine-readable.

```
canoxide-cli [global options] <command> [arguments] [options]
```

Options may stand anywhere on the line, as `--name value` or
`--name=value`. Indexes and CAN identifiers are hex (the `0x` prefix is
optional); sub-indexes and other numbers are decimal or `0x` hex.

Start here:

```
canoxide-cli help            # usage text
canoxide-cli help --json     # the full schema: commands, options, "sends", outputs, exit codes
canoxide-cli help sdo write  # one command
```

## Safety model

Read this before you use `--adapter pcan`, `canfox`, `kvaser` or `slcan`.

1. **The simulator is the default.** Without `--adapter pcan`, `canfox`, `kvaser` or `slcan` every command
   runs against the built-in simulated node. Nothing touches hardware.
2. **Nothing is sent unless a command transmits.** The schema marks each
   command with `"sends": true` or `false`. `monitor`, `eds ...`, `help`,
   `version` and `adapters` send nothing. Even an SDO *read* transmits (a
   request frame), so it is `"sends": true`.
3. **`--listen-only` refuses every sending command** with exit code 2 and
   kind `listen_only`, *before* the adapter is opened. Use it to watch a
   live machine without any risk of a frame from this tool.
4. **Bounded commands.** `monitor` and `watch` need `--duration` or
   `--count` (a replay ends with its log file). A cyclic `send` needs `--count` (at most 100000).
5. **No state between commands** unless you give `--data-dir`. Each
   command opens the adapter, does its work and closes it. The simulator
   restarts with each command; use `shell` for multi-step work.
6. **EDS limits** (LowLimit/HighLimit) are checked before an SDO write is
   sent, when the type comes from the EDS. `--type` skips that check.
7. **`--yes` for the two commands that can do the most harm.** `program
   download` (it stops and replaces the program of the node) and `drive
   command enable` (the drive can move) send nothing without `--yes`: they
   exit with code 2 and say what they would do.
8. **No network.** The CLI never makes a network request; the update
   check (one HTTPS GET to GitHub) is in the GUI only. An SLCAN adapter
   over TCP connects only to the `host:port` that you give.

On a real bus, act like a careful engineer: `scan` and `info` first, read
before you write, and know what an NMT command does to a running machine.
`nmt ... --all` reaches every node.

## Simulator-first workflow

Develop every script against the simulator, then add `--adapter pcan` (or `canfox`, `kvaser`, `slcan`)
(and `--channel`, `--bitrate`, `--node`) only when the script is right.

The simulated node: node-ID = `--node` (default 1), boots pre-operational,
heartbeat every 1000 ms, TPDOs only when operational. Its dictionary
(`canoxide-cli od dump`) has, among others:

| Object | Content |
|---|---|
| 0x1008 | device name (VISIBLE_STRING) |
| 0x1017 | producer heartbeat time, 1000 ms |
| 0x1018:1..4 | vendor 0x69B, product 1, revision 0x10000, serial 0x12345678 |
| 0x2000 | "Test string" (VISIBLE_STRING rw) |
| 0x2001 | Counter (UNSIGNED16 ro, in TPDO2) |
| 0x2002 | Set point (INTEGER32 rw, -100, limits -1000..1000, in TPDO2) |
| 0x2003 / 0x2100 | domains (0x2100: 4096 bytes, for block transfer) |
| 0x2004 | Command (UNSIGNED8 write-only) |
| TPDO1 0x181, TPDO2 0x281, RPDO1 0x201 | the default PDOs |

Give `--eds file.eds` to run the simulator with your device's dictionary.

## Output

Success: `{"ok": true, "command": "<name>", ...fields}`, one line.

Error:

```
{"ok":false,"error":{"kind":"<kind>","message":"<text>","abort_code":"0x........" or null}}
```

A failed multi-step command (PDO change, heartbeat set, DCF restore) adds a
`"detail"` object next to `"error"` with the step report.

`monitor`, `watch` and `shell` write JSON Lines: one object per line, and
for `monitor` and `watch` a final result line. `--text` gives tables and
`key: value` lines for people.

## Exit codes

| Code | Meaning | Error kinds |
|---|---|---|
| 0 | ok | |
| 1 | device or protocol error | `abort` (SDO abort, with `abort_code`), `timeout`, `protocol`, `device`, `cancelled` |
| 2 | usage error | `usage`, `listen_only`, `file`, `eds`, `pdo_check` (the PDO check found an error; nothing written) |
| 3 | adapter or driver error | `adapter` |

A `shell` run returns the code of the last failed line, else 0.

## Commands

| Group | Commands |
|---|---|
| info | `help [--json]`, `version`, `adapters` |
| EDS files | `eds info`, `eds objects`, `eds pdos` |
| DBC files | `dbc info` |
| J1939 | `j1939 decode-id <id>` (no bus) |
| network | `scan`, `info`, `nmt`, `sync`, `send`, `monitor`, `heartbeat set` |
| objects | `sdo read`, `sdo write`, `od dump`, `watch` |
| PDOs | `pdo read`, `pdo enable`, `pdo disable`, `pdo set [--force]`, `pdo map [--force]` |
| LSS | `lss switch-global`, `lss switch-selective`, `lss inquire-identity`, `lss inquire-node-id`, `lss set-node-id`, `lss set-bit-timing`, `lss activate-bit-timing`, `lss store`, `lss fastscan`, `lss identify` |
| DCF | `dcf save`, `dcf restore`, `dcf compare` |
| EDS check | `check [--from] [--to] [--junit out.xml] [--all]` |
| emergencies | `emcy history`, `emcy clear` |
| parameters | `store [--sub n]`, `restore-defaults [--sub n]` |
| program download (CiA 302-3) | `program info [--program p]`, `program download --file f [--program p] [--no-stop] [--clear] [--no-start] [--yes]` |
| drive (CiA 402) | `drive status`, `drive command <command> [--yes]` (shutdown, switch-on, enable, disable-voltage, quick-stop, disable-operation, fault-reset) |
| scripting | `shell [--script file]` |

`help --json` has the options and the output fields of each command.

### Raw CAN IDs (11 and 29 bit)

A standard (11-bit) and an extended (29-bit) frame with the same number are
two different messages. `send <id>` and `monitor --id lo[-hi]` read an ID in
hex (`0x` optional): a value above `7FF` is extended, a trailing `x`
(`123x`) makes a small value extended, and 8 digits (`00000123`) are
extended too. More than 29 bits is a usage error. `send --ext` still works
and makes the ID extended.

The output writes an ID as `0x` and 3 hex digits for a standard ID
(`"id":"0x123"`) and 8 hex digits for an extended ID
(`"id":"0x00000123"`, `"id":"0x18FEF100"`), with the separate flag `ext`
(`send`, `monitor`) or `extended` (`dbc info`). An extended frame is never
decoded as CANopen. `monitor` decodes it as J1939 (`"type":"J1939"`,
`"class":"j1939"`, `"node"` = the source address), with `--ext-raw` as a
raw frame (`"type":"Ext"`, `"class":"other"`); see J1939 below.

`monitor --id` sorts every standard ID before every extended ID:
`--id 180-1FF` holds standard frames only, `--id 18FEF100-18FEF1FF`
extended frames only, `--id 0-1FFFFFFF` every frame.

```
> canoxide-cli send 18FEF100 "01 02"
{"command":"send","data":"01 02","dlc":2,"ext":true,"id":"0x18FEF100","ok":true,"period_ms":null,"rtr":false,"sent":1}
```

### J1939 (receive only)

`monitor` decodes every 29-bit frame as SAE J1939 unless `--ext-raw` is
given. A J1939 frame gets the field `j1939` with `pgn`, `name` (the short
name of the parameter group, or `null`), `priority`, `sa` (source
address) and `da` (destination: the PDU1 address, else 255). Its `text`
has the PGN, the priority, the destination and the decoded detail; the
transport protocol (TP.CM / TP.DT, BAM and RTS/CTS) is followed, and the
last packet adds `complete: PGN n NAME, k bytes` with the DM1 / DM2
faults. `--class j1939` keeps only these frames. A frame with EDP and DP
both set is not J1939 (`"type":"Ext"`). Nothing is sent: no address
claim, no request, no CTS.

`j1939 decode-id <id>` splits a 29-bit ID (hex, `0x` optional) without a
bus: `id`, `j1939`, `pgn`, `name`, `priority`, `sa`, `da`, `pdu` (1 or 2),
`pf`, `ps`, `edp`, `dp`. With EDP and DP both set the answer is
`{"id", "j1939":false, "edp", "dp", "note"}`. More than 29 bits or no ID is
a usage error.

```
> canoxide-cli j1939 decode-id 0x18FEF100
{"command":"j1939 decode-id","da":255,"dp":false,"edp":false,"id":"0x18FEF100","j1939":true,"name":"CCVS","ok":true,"pdu":2,"pf":254,"pgn":65265,"priority":6,"ps":241,"sa":0}
```

Global options: `--adapter sim|pcan|canfox|kvaser|slcan|replay`, `--file log`,
`--speed max|factor`, `--channel usb1..usb8` (PCAN), a net number (CANfox, default 105),
a channel number (Kvaser, default 0) or `COM<n>` / `host:port` (SLCAN, no default),
`--bitrate 10k..1M`, `--node 1..127`, `--eds file`, `--listen-only`,
`--timeout ms` (SDO), `--data-dir dir`, `--device name`, `--text`.

`--data-dir` is the GUI data folder; with `--device name` the node-ID, EDS
and bit rate come from that device of the library, and `pdo read` saves the
read configuration there.

## LSS warning

LSS (CiA 305) changes how a device is addressed. `lss set-node-id`,
`lss set-bit-timing` with `lss store`, and above all
`lss activate-bit-timing` (every node on the bus changes its bit rate) can
make a device or the whole network unreachable until it is configured
again. `lss switch-global configuration` puts **every** LSS slave into
configuration state. Use LSS only on a bench, with one unconfigured device
where you can, and try the sequence in `shell` on the simulator first.

## Worked examples

All output below was captured from the simulator (`--adapter sim`, the
default).

### 1. Find the nodes

```
> canoxide-cli scan --to 5
{"command":"scan","errors":[],"found":1,"from":1,"nodes":[{"device_type":"0x00000191","node":1}],"ok":true,"to":5}
```

### 2. Node overview

```
> canoxide-cli info
{"command":"info","nodes":[{"device_name":"CANoxide simulator","device_type":"0x00000191","error_register":"0x00","last_emcy":null,"node":1,"product_code":"0x00000001","read_errors":[],"revision":"0x00010000","serial":"0x12345678","state":"Boot-up","vendor_id":"0x0000069B"}],"ok":true}
```

### 3. Read an object

`value` is typed (a number, a string or hex bytes); `hex`, `decimal` and
`string` are the three display forms of the GUI; `raw` is the bytes.

```
> canoxide-cli sdo read 0x1018:1
{"command":"sdo read","decimal":"1691","hex":"0x0000069B","index":"0x1018","mode":"auto","name":"Identity object / Vendor ID","node":1,"ok":true,"raw":"9B 06 00 00","size":4,"string":"....","sub":1,"type":"UNSIGNED32","value":1691}
```

### 4. Write an object, and a value outside the EDS limits

```
> canoxide-cli sdo write 0x2002 250
{"command":"sdo write","data":"FA 00 00 00","index":"0x2002","mode":"auto","name":"Set point","node":1,"ok":true,"size":4,"sub":0,"type":"INTEGER32","written":true}

> canoxide-cli sdo write 0x2002 5000      (exit 2, nothing sent)
{"error":{"abort_code":null,"kind":"usage","message":"value '5000': value 5000 above high limit 1000 (EDS limits)"},"ok":false}
```

### 5. An SDO abort

```
> canoxide-cli sdo read 0x5000      (exit 1)
{"error":{"abort_code":"0x06020000","kind":"abort","message":"abort 0x06020000: Object does not exist in the object dictionary"},"ok":false}
```

### 6. Listen-only refuses a send

```
> canoxide-cli --listen-only nmt start      (exit 2)
{"error":{"abort_code":null,"kind":"listen_only","message":"'nmt' transmits on the bus; refused with --listen-only (nothing was opened or sent)"},"ok":false}
```

### 7. Monitor the bus

```
> canoxide-cli monitor --duration 1.5
{"class":"heartbeat","data":"00","dir":"Rx","dlc":1,"ext":false,"id":"0x701","node":1,"rtr":false,"t":4e-6,"text":"Boot-up","type":"Heartbeat"}
{"class":"heartbeat","data":"7F","dir":"Rx","dlc":1,"ext":false,"id":"0x701","node":1,"rtr":false,"t":1.001122,"text":"Heartbeat Pre-operational","type":"Heartbeat"}
{"command":"monitor","duration_s":1.501,"frames":2,"ok":true}
```

`t` is seconds from the open of the adapter. Filters: `--filter-node`,
`--class heartbeat,emcy`, `--id 180-1FF` (standard frames), `--id 18FEF100-18FEF1FF`
(extended frames; see "Raw CAN IDs").

### 8. Read and change a PDO

```
> canoxide-cli pdo read --only tpdo1
{"command":"pdo read","complete":true,"errors":[],"node":1,"ok":true,"pdos":[{"cob_id":"0x181","event_timer_ms":200,"extended":false,"inhibit_100us":0,"kind":"TPDO","mapping":[{"bits":8,"index":"0x6000","name":"Read input 8 bit / Inputs 1-8","sub":1},{"bits":8,"index":"0x6000","name":"Read input 8 bit / Inputs 9-16","sub":2}],"name":"tpdo1","number":1,"rtr_allowed":true,"sync_start":null,"trans_type":255,"valid":true}],"read_time":1795047599,"read_time_text":"2026-11-19 00:19:59 UTC"}

> canoxide-cli pdo set tpdo1 --event 50
{"after":{...,"event_timer_ms":50,...},"before":{...,"event_timer_ms":200,...},"changed":true,"command":"pdo set","node":1,"ok":true,"pdo":"tpdo1","steps":{"cancelled":false,"done":[{"abort_code":null,"data":"81 01 00 80","error":null,"index":"0x1800","node":1,"ok":true,"sub":1,"what":"set TPDO1 invalid"},...],"not_run":[],"ok":true}}
```

(The `before`/`after` objects and the step list are shortened here.) A
change follows the CiA 301 sequence: PDO invalid, write, valid again.
`steps.done` lists each SDO write.

Before it writes, `pdo set` and `pdo map` run the PDO check on the new
configuration: against the EDS (`--eds` or `--device`) and against the
PDOs saved in the library (`pdo read --device`). Errors: more than 64
bits, more than 64 entries (more than 8 when every entry is a whole number
of bytes), an entry of 0 bits, an object that is not in the EDS, a length
that differs from the data type, a COB-ID in a restricted range of CiA 301
(0x000-0x07F, 0x101-0x180, 0x581-0x5FF, 0x601-0x67F, 0x6E0-0x6FF,
0x701-0x7FF), transmission type 252 or 253 (RTR only) on an RPDO.
Warnings: no EDS, an object with `PDOMapping=0`, the wrong access for the
direction, the default COB-ID of another node, a COB-ID that another known
PDO uses, transmission type 241-251 (reserved). The COB-ID checks run only
for a valid PDO. An error stops the command with exit code 2:

```
> canoxide-cli pdo set tpdo1 --cob-id 5A0
{"detail":{"after":{...},"findings":[{"code":"cob_restricted","message":"COB-ID 0x5A0 is in the restricted range 0x581..0x5FF of CiA 301","severity":"error"}],"node":1,"pdo":"tpdo1"},"error":{"abort_code":null,"kind":"pdo_check","message":"tpdo1: the PDO check found 1 error(s): ...; nothing written (--force writes anyway)"},"ok":false}
```

`--force` writes anyway. The result of every change has `findings` (the
warnings, and the errors with `--force`) and `forced` (true when an error
was overruled). The 64-bit limit of `pdo map` stays: `--force` does not
write a mapping of more than 64 bits.

### 9. Save and compare a DCF

```
> canoxide-cli dcf save node1.dcf
{"cancelled":false,"command":"dcf save","failed":[],"file":"node1.dcf","node":1,"not_stored":[{"index":"0x6000","sub":0},{"index":"0x6000","sub":1},{"index":"0x6000","sub":2},{"index":"0x6200","sub":0},{"index":"0x6200","sub":1},{"index":"0x6200","sub":2}],"ok":true,"values":59}

> canoxide-cli dcf compare node1.dcf
{"cancelled":false,"command":"dcf compare","compared":59,"differences":[],"equal":true,"file":"node1.dcf","node":1,"ok":true}
```

`dcf restore node1.dcf --dry-run` lists what a restore would write.

### 10. A multi-step script in one session

`steps.txt`:

```
# set a value, start the node and watch the value in TPDO2
sdo write 0x2002 33
nmt start
watch pdo:0x2002:0 --count 2
nmt stop
exit
```

```
> canoxide-cli shell --script steps.txt
{"command":"sdo write","data":"21 00 00 00","index":"0x2002","mode":"auto","name":"Set point","node":1,"ok":true,"size":4,"sub":0,"type":"INTEGER32","written":true}
{"all":false,"command":"nmt","label":"Start","node":1,"ok":true,"service":"start"}
{"raw":"21 00 00 00","t":1.001,"value":33}
{"raw":"21 00 00 00","t":2.001,"value":33}
{"cob_id":"0x281","command":"watch","index":"0x2002","name":"Set point","node":1,"ok":true,"pdo":"tpdo2","samples":2,"source":"pdo","sub":0}
{"all":false,"command":"nmt","label":"Stop","node":1,"ok":true,"service":"stop"}
```

The shell keeps one adapter open and the simulator keeps its state. A
line may set `--node`, `--timeout` and `--text`; the other global options
are fixed by the `shell` command line. Empty lines and `#` comments are
skipped; `exit` (or the end of the input) ends the shell. A shell can also
read stdin, so a program can keep it open and send one line at a time.

### 11. Check a node against its EDS

`check` reads each entry of the EDS (`--eds`, or the built-in EDS of the
simulator) from the node and compares it: `ok`, `missing`,
`write_only_ok`, `access`, `size`, `value`, `abort`, `timeout` (and
`not_checked` after a cancel or after `--stop-after` time-outs in a row).
It writes nothing. `entries` lists only the entries that are not ok,
unless `--all`. The exit code is 1 when at least one entry failed, so a CI
job can use it; `--junit out.xml` also writes JUnit XML.

```
> canoxide-cli check --junit check.xml
{"cancelled":false,"checked":79,"command":"check","counts":{"abort":0,"access":0,"missing":0,"not_checked":0,"ok":78,"size":0,"timeout":0,"value":0,"write_only_ok":1},"duration_ms":0,"eds_file":"sim_builtin.eds","entries":[],"failed":0,"junit":"check.xml","node":1,"not_checked":0,"ok":true,"ok_entries":79,"passed":true,"skipped_domain":2,"stopped_on_timeouts":false,"summary":"79 checked: 79 ok, 0 failed, 0 warning(s) - sim_builtin.eds, node 1, 0.00 s","warnings":0}
```

Missing, access, size and time-out results fail; a `value` result fails for
0x1000 and 0x1018, else it is a warning; another abort is a warning. On a
failure the result is `{"ok":false,"error":{...},"detail":{...}}` with the
same fields in `detail`.

### 12. The error history (0x1003)

```
> canoxide-cli emcy history
{"command":"emcy history","count":2,"entries":[{"code":"0x8130","info":"0x0011","number":1,"raw":"0x00118130","text":"Life guard or heartbeat error"},{"code":"0x5000","info":"0x0000","number":2,"raw":"0x00005000","text":"Device hardware"}],"node":1,"ok":true}

> canoxide-cli emcy clear
{"cleared":true,"command":"emcy clear","index":"0x1003","node":1,"ok":true,"sub":0}
```

Entry 1 is the newest. `code` is the low 16 bits of the entry, `info` the
high 16 bits (manufacturer specific). `emcy clear` writes 0 to 0x1003:00.

### 13. Store parameters and restore the defaults

```
> canoxide-cli store
{"command":"store","index":"0x1010","name":"All parameters","node":1,"ok":true,"stored":true,"sub":1}

> canoxide-cli restore-defaults --sub 3
{"command":"restore-defaults","index":"0x1011","name":"Application parameters","node":1,"note":"The defaults are active after a reset node or a power cycle.","ok":true,"restored":true,"sub":3}
```

`store` writes "save" (0x65766173) to 0x1010:sub, `restore-defaults`
writes "load" (0x64616F6C) to 0x1011:sub. `--sub` 1 = all parameters (the
default), 2 = communication, 3 = application, 4 and up manufacturer
specific. A node that refuses aborts with 0x08000020 (exit 1). Both are
refused with `--listen-only`, as `check`, `emcy history` and `emcy clear`
are.

### 14. What a DBC import gives

```
> canoxide-cli dbc info bus.dbc
{"command":"dbc info","file":"bus.dbc","message_count":1,"messages":[{"dlc":4,"extended":false,"id":"0x3A0","name":"Level","signals":[{"bits":16,"byte_order":"intel","factor":0.5,"kind":"unsigned","name":"depth","offset":-10.0,"start_bit":8,"unit":"mm"}]}],"multiplexed":1,"ok":true,"signals":1,"skipped":["Level signal Mux: multiplexed signals are not supported"]}
```

`dbc info` reads the file only (no bus). It gives the messages and
signals that the GUI's **Import DBC...** turns into user signals, and what
it skips: multiplexed signals, float64 signals and IDs that are not CAN
IDs. The start bit is the DBC start bit (Intel: the least significant bit;
Motorola: the most significant bit). A file that cannot be read fails with
kind `file`.

### 15. Program download (CiA 302-3)

```
> canoxide-cli program info
{"command":"program info","control":1,"flash_error":0,"flash_error_text":"no error","flash_in_progress":false,"flash_status":"0x00000000","identification":"0x5A5A0001","node":1,"objects":[...],"ok":true,"program":1}

> canoxide-cli program download --file app.bin
{"detail":{"crc32":"0x5D3DE8ED","file":"app.bin","node":1,"plan":["Read the program identification (0x1F56) sub 1","Stop the program (0x1F51 = 0) sub 1","Download the file (0x1F50) sub 1: 1024 bytes (Auto transfer)",...],"program":1,"sha256":"e918...056d","size":1024},"error":{"abort_code":null,"kind":"usage","message":"program download stops and replaces the program of node 1; nothing was sent. Add --yes to run it (the plan is in detail.plan)"},"ok":false}

> canoxide-cli program download --file app.bin --yes
{"bytes_done":1024,"cancelled":false,"command":"program download","crc32":"0x5D3DE8ED","new_id":"0x5D3DE8ED","ok":true,"old_id":"0x5A5A0001",...,"steps":[{"abort_code":null,"detail":"0x5A5A0001","name":"Read the program identification (0x1F56)","state":"ok","step":"read_old_id"},...]}
```

`program info` reads 0x1F56 (program identification), 0x1F57 (flash
status: bit 0 = in progress, bits 1..7 = error code) and 0x1F51 (program
control) at sub `--program` (default 1).

`program download` sends nothing without `--yes`: it exits with code 2
and gives the plan, the size and the SHA-256 of the file in `detail`. With
`--yes` it runs these steps; each one is reported with the state `ok`,
`skipped`, `failed`, `cancelled` or `not_run`:

1. read 0x1F56 (the old identification);
2. stop: 0x1F51 = 0 (`--no-stop` skips it; a node that refuses a download
   while its program runs aborts the download);
3. clear: 0x1F51 = 3 (only with `--clear`);
4. download the file to 0x1F50 (`--mode auto|segmented|block`);
5. poll 0x1F57 until bit 0 is 0 (time-out 30 s; skipped when the node has
   no 0x1F57);
6. check the error code of 0x1F57;
7. start: 0x1F51 = 1 (`--no-start` skips it and the next step);
8. wait for the boot-up message (time-out 10 s);
9. read 0x1F56 again.

A failed step stops the sequence; nothing is tried again. The steps that
did not run have the state `not_run`. The exit code is then 1 (kind
`abort` with the abort code, or `device`), and the report is in `detail`.
`crc32` is the CRC-32 of the file; the simulator reports it as its new
program identification. Never download to a machine in operation.

### 16. A CiA 402 drive

```
> canoxide-cli drive status
{"allowed":["shutdown"],"bits":{"internal_limit_active":false,"remote":true,"target_reached":false,"voltage_enabled":false,"warning":false},"command":"drive status","error_code":"0x0000","mode":3,"mode_name":"3 Profile velocity (pv)","na":[],"node":1,"ok":true,"position":0,"state":"switch_on_disabled","state_name":"Switch on disabled","statusword":"0x0240","torque":0,"velocity":0}

> canoxide-cli drive command shutdown
{"action":"shutdown","command":"drive command","controlwords":["0x0006"],"node":1,"ok":true,"state":"ready_to_switch_on","state_name":"Ready to switch on","statusword":"0x0621","steps":{...}}

> canoxide-cli drive command enable
{"error":{"abort_code":null,"kind":"usage","message":"'enable' (Enable operation) can make the drive of node 1 move; nothing was sent. Check that the drive can move safely, then add --yes"},"ok":false}
```

`drive status` reads 0x6041, 0x6061, 0x6064, 0x606C, 0x6077 and 0x603F.
An object that the drive does not have (abort 0x06020000 or 0x06090011) is
listed in `na` and its field is null. `allowed` lists the commands that
the state machine accepts in the current state.

`drive command` writes the controlword 0x6040 (shutdown 0x06, switch-on
0x07, enable 0x0F, disable-voltage 0x00, quick-stop 0x02,
disable-operation 0x07, fault-reset 0x80 then 0x00) and reads the state
after it. `enable` (Enable operation) can make the drive move: without
`--yes` it exits with code 2 and sends nothing. The tool does not check
that a command is allowed; the drive decides. Each command is a separate
session, so use `shell` for a sequence on the simulator.

### Text output

```
> canoxide-cli sdo read 0x1017 --text
command: sdo read
decimal: 1000
hex: 0x03E8
index: 0x1017
mode: auto
name: Producer heartbeat time
node: 1
ok: true
raw: E8 03
size: 2
string: ..
sub: 0
type: UNSIGNED16
value: 1000
```

## On a real bus

```
canoxide-cli adapters --adapter pcan                       # the channels (none opened)
canoxide-cli --adapter pcan --listen-only monitor --duration 10
canoxide-cli --adapter pcan --channel usb1 --bitrate 250k scan
canoxide-cli --adapter pcan --node 5 --eds dev.eds sdo read 0x1008
```

The PEAK driver (`PCANBasic.dll`, 64 bit) must be installed; without it a
command with `--adapter pcan` ends with exit code 3, kind `adapter`.

The ifm CANfox works through the Sontheim / ifm CANfox driver (`SIECA164.DLL`,
64 bit, found through `CANOXIDE_SIECA_DLL`, the Windows search, or the
Sontheim folder under Program Files). `--channel` is the net number; a
CANfox is net 105. Tested so far on one CANfox at 500 kbit/s: the net
list, reception and one SDO read.

```
canoxide-cli adapters --adapter canfox                         # the nets (none opened)
canoxide-cli --adapter canfox --channel 105 --bitrate 250k --listen-only monitor --duration 10
```

Without the driver a command with `--adapter canfox` ends with exit code 3,
kind `adapter`. When the driver does not answer (seen after a failed
CANfox firmware update) the command ends after about 3 s with the same exit
code; close CoDeSys and restart the PC. `open` never forces the bitrate:
when another program (for example CoDeSys) owns the net and the bitrate
cannot be set, the command fails and says so.

### Kvaser (not tested on hardware)

The Kvaser adapter works through CANlib (`canlib32.dll` of the Kvaser
drivers, from `System32` or the exe folder). `--channel` is the CANlib
channel number (default 0); the Kvaser virtual channels work too. The
channel is opened shared, listen only uses the silent driver mode, and
every driver call has a time-out of about 3 s. Not tested on hardware:
please report results.

```
canoxide-cli adapters --adapter kvaser                       # the channels (none opened)
canoxide-cli --adapter kvaser --channel 0 --bitrate 500k --listen-only monitor --duration 10
```

Without the driver a command with `--adapter kvaser` ends with exit code 3,
kind `adapter` ("Kvaser driver not found: install the Kvaser drivers").

### SLCAN (not tested on hardware)

The SLCAN adapter talks the Lawicel ASCII protocol (CANable or candleLight
with slcan firmware, Lawicel CANUSB, USBtin, network gateways). `--channel`
is a COM port or `host:port`; a channel with a colon and a port number is
TCP. There is no default: without `--channel` a command that opens the
adapter ends with exit code 2, kind `usage`, and so does a channel text
that is neither. Opening sends `C`, `V`, `S<n>` and `O` (`L` with
`--listen-only`), never a CAN frame; a device that does not answer `V`
ends with exit code 3, kind `adapter`. Not tested on hardware: please
report results.

```
canoxide-cli adapters --adapter slcan                        # the COM ports (none opened)
canoxide-cli --adapter slcan --channel COM5 --bitrate 250k --listen-only monitor --duration 10
canoxide-cli --adapter slcan --channel 192.168.1.10:2001 --node 5 sdo read 0x1008
```

## Replay of a log file

`--adapter replay --file <log>` feeds `monitor` and `watch pdo:` from a log
file instead of a bus. Nothing is sent: the replay adapter refuses every
frame, and every other command that needs a bus fails with exit code 2,
kind `usage`, before the file is opened.

```
canoxide-cli --adapter replay --file trace.csv monitor
canoxide-cli --adapter replay --file can0.log --speed 1 monitor --class tpdo --text
canoxide-cli --adapter replay --file trace.csv --eds dev.eds watch pdo:0x2001:0
canoxide-cli --adapter replay --file bench.trc monitor --class tpdo
canoxide-cli --adapter replay --file bench.asc monitor --text
```

- Formats: the trace CSV that CANoxide writes (the continuous log and
  "Save trace...", also the files of 0.2.0; data in hex or decimal),
  candump logs (`(1700000000.123456) can0 123#DEADBEEF`, `#R` for a remote
  frame), PEAK `.trc` files (versions 1.0 to 1.3 and 2.0 to 2.1, also 3.0;
  `$COLUMNS` is read; without `$FILEVERSION` the file is 1.0) and Vector
  `.asc` files (`base hex|dec`, `timestamps absolute|relative`, an `x`
  after an extended ID, `r` for a remote frame). The format comes from the
  first line, not from the file name.
- Records that are no classic CAN frame (CAN FD frames, error frames,
  status, event and trigger lines, `TxRq`) are skipped and counted; the
  count goes to stderr. They are not bad lines. A file with only such
  records fails with kind `file`.
- Time: the first frame is at 0 s; `t` is the log time. A time that goes
  back is raised to the one before it.
- `--speed max` (the default) reads the file as fast as possible;
  `--speed 1` paces the frames in real time, `--speed 4` four times faster.
- `monitor` and `watch` end at the end of the log, so `--duration` and
  `--count` are optional. `--duration` counts log time.
- The `dir` column of a CANoxide CSV is kept (`tx` frames stay `tx`);
  candump frames are `rx`.
- A bad line is skipped; the count and the first error (with its line
  number) go to stderr. A file with no good frame fails with kind `file`.
- `watch pdo:` takes the TPDO mapping from the defaults of `--eds`
  (the node cannot be asked). An SDO `watch` is refused.
