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

Read this before you use `--adapter pcan`.

1. **The simulator is the default.** Without `--adapter pcan` every command
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

On a real bus, act like a careful engineer: `scan` and `info` first, read
before you write, and know what an NMT command does to a running machine.
`nmt ... --all` reaches every node.

## Simulator-first workflow

Develop every script against the simulator, then add `--adapter pcan`
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
| 2 | usage error | `usage`, `listen_only`, `file`, `eds` |
| 3 | adapter or driver error | `adapter` |

A `shell` run returns the code of the last failed line, else 0.

## Commands

| Group | Commands |
|---|---|
| info | `help [--json]`, `version`, `adapters` |
| EDS files | `eds info`, `eds objects`, `eds pdos` |
| network | `scan`, `info`, `nmt`, `sync`, `send`, `monitor`, `heartbeat set` |
| objects | `sdo read`, `sdo write`, `od dump`, `watch` |
| PDOs | `pdo read`, `pdo enable`, `pdo disable`, `pdo set`, `pdo map` |
| LSS | `lss switch-global`, `lss switch-selective`, `lss inquire-identity`, `lss inquire-node-id`, `lss set-node-id`, `lss set-bit-timing`, `lss activate-bit-timing`, `lss store`, `lss fastscan`, `lss identify` |
| DCF | `dcf save`, `dcf restore`, `dcf compare` |
| scripting | `shell [--script file]` |

`help --json` has the options and the output fields of each command.

Global options: `--adapter sim|pcan|replay`, `--file log`,
`--speed max|factor`, `--channel usb1..usb8`,
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
`--class heartbeat,emcy`, `--id 180-1FF`.

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

## Replay of a log file

`--adapter replay --file <log>` feeds `monitor` and `watch pdo:` from a log
file instead of a bus. Nothing is sent: the replay adapter refuses every
frame, and every other command that needs a bus fails with exit code 2,
kind `usage`, before the file is opened.

```
canoxide-cli --adapter replay --file trace.csv monitor
canoxide-cli --adapter replay --file can0.log --speed 1 monitor --class tpdo --text
canoxide-cli --adapter replay --file trace.csv --eds dev.eds watch pdo:0x2001:0
```

- Formats: the trace CSV that CANoxide writes (the continuous log and
  "Save as CSV", also the files of 0.2.0; data in hex or decimal) and
  candump logs (`(1700000000.123456) can0 123#DEADBEEF`, `#R` for a remote
  frame). The format comes from the first line.
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
