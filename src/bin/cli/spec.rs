//! The command table: names, options, whether a command transmits, output
//! and examples. `help --json` prints it; the parser checks options with it.

use serde_json::{json, Value};

/// One option: name (without `--`), value placeholder (`None` = a flag),
/// help text.
pub type Opt = (&'static str, Option<&'static str>, &'static str);

pub struct Spec {
    pub name: &'static str,
    /// Positional arguments after the name.
    pub args: &'static str,
    pub summary: &'static str,
    /// True when the command transmits on the bus.
    pub sends: bool,
    /// True when the command needs an open adapter.
    pub bus: bool,
    pub options: &'static [Opt],
    pub output: &'static str,
    pub example: &'static str,
}

pub const GLOBAL: &[Opt] = &[
    ("adapter", Some("sim|pcan"), "Adapter. Default sim (the built-in simulator; nothing touches hardware)."),
    ("channel", Some("usb1..usb8"), "PCAN-USB channel. Default usb1."),
    ("bitrate", Some("10k..1M"), "Bit rate: 10k 20k 50k 125k 250k 500k 800k 1M. Default 250k."),
    ("node", Some("1..127"), "Node-ID of the device. Default 1. With the simulator, also the ID of the simulated node."),
    ("eds", Some("file"), "EDS/DCF: names and types in the output, and the dictionary of the simulated node."),
    ("listen-only", None, "Open the channel listen-only. Every command that transmits fails with exit code 2 before anything is opened."),
    ("timeout", Some("ms"), "SDO timeout. Default 1000."),
    ("data-dir", Some("dir"), "Data folder of the GUI (device library). Without it the tool keeps no state on disk."),
    ("device", Some("name"), "Device of the library in --data-dir: node-ID, EDS and bit rate from it."),
    ("text", None, "Human-readable text instead of JSON."),
];

const MODE: Opt = ("mode", Some("auto|segmented|block"), "SDO transfer mode. auto = expedited or segmented; block only when asked.");
const TYPE: Opt = ("type", Some("type"), "Data type: u8 u16 u24 u32 u64 i8 i16 i32 i64 f32 f64 bool string domain, a CiA name (UNSIGNED32) or a code (0x0007). Default: from the EDS.");
const BLOCK: Opt = ("block-size", Some("1..127"), "Block size of a block transfer. Default 127.");
const ALL: Opt = ("all", None, "All nodes: NMT broadcast (node 0).");
const NODES: Opt = ("nodes", Some("list"), "Node-IDs, comma separated (default --node).");
const DURATION: Opt = ("duration", Some("s"), "Stop after this many seconds.");
const COUNT: Opt = ("count", Some("n"), "Stop after this many samples / frames.");
const LSS_TIMEOUT: Opt = ("lss-timeout", Some("ms"), "LSS answer timeout. Default 100.");

pub const COMMANDS: &[Spec] = &[
    Spec {
        name: "help",
        args: "[command]",
        summary: "Usage text, or with --json the full machine-readable schema of all commands.",
        sends: false,
        bus: false,
        options: &[("json", None, "Print the schema as JSON.")],
        output: "Text, or the schema object.",
        example: "canopen-cli help --json",
    },
    Spec {
        name: "version",
        args: "",
        summary: "Program name and version.",
        sends: false,
        bus: false,
        options: &[],
        output: "{name, version}",
        example: "canopen-cli version",
    },
    Spec {
        name: "adapters",
        args: "",
        summary: "The adapters. With --adapter pcan also the PCAN channels (the PCAN driver is asked; no channel is opened).",
        sends: false,
        bus: false,
        options: &[],
        output: "{adapters: [{name, channels: [{name, handle, available}]}]}",
        example: "canopen-cli adapters",
    },
    Spec {
        name: "eds info",
        args: "<file>",
        summary: "File and device information of an EDS or DCF.",
        sends: false,
        bus: false,
        options: &[],
        output: "{file, objects, entries, file_info, device_info, is_dcf, node_id, baudrate_kbit, warnings}",
        example: "canopen-cli eds info io.eds",
    },
    Spec {
        name: "eds objects",
        args: "<file>",
        summary: "All entries of an EDS or DCF.",
        sends: false,
        bus: false,
        options: &[],
        output: "{objects: [{index, sub, name, type, access, default, pdo_mapping}]}",
        example: "canopen-cli eds objects io.eds",
    },
    Spec {
        name: "eds pdos",
        args: "<file>",
        summary: "The default PDO configuration of an EDS (for --node).",
        sends: false,
        bus: false,
        options: &[],
        output: "{pdos: [PdoConfig]}",
        example: "canopen-cli eds pdos io.eds --node 5",
    },
    Spec {
        name: "scan",
        args: "",
        summary: "Find nodes: read 0x1000 of each node-ID in the range.",
        sends: true,
        bus: true,
        options: &[("from", Some("id"), "First node-ID. Default 1."), ("to", Some("id"), "Last node-ID. Default 127."), ("scan-timeout", Some("ms"), "Wait per node. Default 100.")],
        output: "{nodes: [{node, device_type}], errors: [{node, error}]}",
        example: "canopen-cli scan --to 10",
    },
    Spec {
        name: "info",
        args: "",
        summary: "Node overview: device type, error register, identity, device name, NMT state, last EMCY.",
        sends: true,
        bus: true,
        options: &[NODES, ("all", None, "Scan 1..127 first and read every node found.")],
        output: "{nodes: [{node, state, device_type, error_register, vendor_id, product_code, revision, serial, device_name, last_emcy, read_errors}]}",
        example: "canopen-cli info --node 5",
    },
    Spec {
        name: "sdo read",
        args: "<index>[:sub]",
        summary: "Read an object. Index in hex (0x prefix optional), sub decimal or 0x hex.",
        sends: true,
        bus: true,
        options: &[TYPE, MODE, BLOCK, ("out", Some("file"), "Write the raw bytes to this file instead of the JSON value.")],
        output: "{node, index, sub, name, type, size, value, hex, decimal, string, raw, mode} or {.., file, size}",
        example: "canopen-cli sdo read 0x1008",
    },
    Spec {
        name: "sdo write",
        args: "<index>[:sub] [value]",
        summary: "Write an object. The value is parsed by its type (numbers: decimal or 0x hex; strings as is; domains as hex bytes).",
        sends: true,
        bus: true,
        options: &[TYPE, MODE, BLOCK, ("in", Some("file"), "Write the bytes of this file (no value argument).")],
        output: "{node, index, sub, name, mode, type, size, data, written: true} or {.., file, size, written}",
        example: "canopen-cli sdo write 0x1017 500 --type u16",
    },
    Spec {
        name: "od dump",
        args: "",
        summary: "Read every readable object of the EDS (the built-in dictionary for the simulator without --eds).",
        sends: true,
        bus: true,
        options: &[],
        output: "{node, read, failed, objects: [{index, sub, name, type, value, hex, error, abort_code}]}",
        example: "canopen-cli od dump",
    },
    Spec {
        name: "nmt",
        args: "<start|stop|preop|reset-node|reset-comm>",
        summary: "Send an NMT command to --node, or to all nodes with --all.",
        sends: true,
        bus: true,
        options: &[ALL],
        output: "{service, label, node, all}",
        example: "canopen-cli nmt start --all",
    },
    Spec {
        name: "sync",
        args: "",
        summary: "Send one SYNC frame (0x080).",
        sends: true,
        bus: true,
        options: &[],
        output: "{sent: 1}",
        example: "canopen-cli sync",
    },
    Spec {
        name: "send",
        args: "<id> [hex bytes]",
        summary: "Send a raw frame (id in hex; data as hex bytes, \"01 02\" or 0102). With --period, --count frames.",
        sends: true,
        bus: true,
        options: &[
            ("ext", None, "29-bit identifier."),
            ("rtr", None, "Remote frame (with --dlc)."),
            ("dlc", Some("0..8"), "DLC of a remote frame."),
            ("period", Some("ms"), "Send --count frames at this period."),
            ("count", Some("n"), "Number of frames (needed with --period, at most 100000)."),
        ],
        output: "{id, ext, rtr, data, sent}",
        example: "canopen-cli send 0x123 \"01 02 03\"",
    },
    Spec {
        name: "monitor",
        args: "",
        summary: "Receive and decode frames. Bounded: --duration or --count is required. Sends nothing.",
        sends: false,
        bus: true,
        options: &[
            DURATION,
            COUNT,
            ("filter-node", Some("id"), "Only frames of this node."),
            ("class", Some("list"), "Only these classes: nmt sync emcy time tpdo rpdo sdo heartbeat lss other (comma separated)."),
            ("id", Some("lo[-hi]"), "Only identifiers in this range (hex)."),
        ],
        output: "JSON Lines: one {t, dir, id, ext, rtr, dlc, data, type, class, node, text} per frame, then {ok, command, frames, duration_s}",
        example: "canopen-cli monitor --duration 2",
    },
    Spec {
        name: "pdo read",
        args: "",
        summary: "Read the PDO configuration of the node over SDO. With --device it is saved in the library.",
        sends: true,
        bus: true,
        options: &[
            ("stop-after", Some("n"), "Consecutive missing PDOs that end a direction. Default 4."),
            ("only", Some("pdo"), "Read only this PDO (rpdo1, tpdo3, ...)."),
        ],
        output: "{node, read_time, read_time_text, complete, errors, pdos: [{kind, number, cob_id, valid, extended, rtr_allowed, trans_type, inhibit_100us, event_timer_ms, sync_start, mapping: [{index, sub, bits}]}]}",
        example: "canopen-cli pdo read",
    },
    Spec {
        name: "pdo enable",
        args: "<rpdoN|tpdoN|all|all-rpdo|all-tpdo>",
        summary: "Set the valid bit of a PDO (CiA 301 sequence over SDO).",
        sends: true,
        bus: true,
        options: &[],
        output: "{steps: {done: [{step, ok, error, abort_code}], not_run, ok}}",
        example: "canopen-cli pdo enable tpdo1",
    },
    Spec {
        name: "pdo disable",
        args: "<rpdoN|tpdoN|all|all-rpdo|all-tpdo>",
        summary: "Clear the valid bit of a PDO.",
        sends: true,
        bus: true,
        options: &[],
        output: "{steps: {...}}",
        example: "canopen-cli pdo disable tpdo1",
    },
    Spec {
        name: "pdo set",
        args: "<rpdoN|tpdoN>",
        summary: "Change communication parameters of one PDO (invalid, write, valid again).",
        sends: true,
        bus: true,
        options: &[
            ("cob-id", Some("id"), "New COB-ID (hex)."),
            ("type", Some("0..255"), "Transmission type."),
            ("inhibit", Some("n"), "Inhibit time in 100 us."),
            ("event", Some("ms"), "Event timer in ms."),
            ("sync-start", Some("0..240"), "SYNC start value."),
        ],
        output: "{before, after, steps: {...}}",
        example: "canopen-cli pdo set tpdo1 --event 50",
    },
    Spec {
        name: "pdo map",
        args: "<rpdoN|tpdoN> <index:sub:bits>...",
        summary: "Replace the mapping of one PDO (index hex, sub and bits decimal).",
        sends: true,
        bus: true,
        options: &[],
        output: "{before, after, steps: {...}}",
        example: "canopen-cli pdo map tpdo2 0x2001:0:16 0x2002:0:32",
    },
    Spec {
        name: "heartbeat set",
        args: "<ms>",
        summary: "Write 0x1017 (producer heartbeat time) on --node, --nodes or every node found by a scan (--all).",
        sends: true,
        bus: true,
        options: &[NODES, ("all", None, "Scan 1..127 first and set every node found.")],
        output: "{ms, nodes, steps: {...}}",
        example: "canopen-cli heartbeat set 1000",
    },
    Spec {
        name: "watch",
        args: "<index:sub | pdo:index:sub>",
        summary: "Sample a value: an object read by SDO at --period, or a mapped PDO object taken from the bus (pdo:). Bounded: --duration or --count.",
        sends: true,
        bus: true,
        options: &[DURATION, COUNT, ("period", Some("ms"), "SDO read period. Default 100, minimum 50."), TYPE],
        output: "JSON Lines: one {t, value, raw} per sample, then {ok, command, samples}",
        example: "canopen-cli watch 0x2002:0 --count 5",
    },
    Spec {
        name: "lss switch-global",
        args: "<configuration|waiting>",
        summary: "LSS switch state global (all LSS slaves).",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, done: true}",
        example: "canopen-cli lss switch-global configuration",
    },
    Spec {
        name: "lss switch-selective",
        args: "<vendor> <product> <revision> <serial>",
        summary: "LSS switch state selective: the slave with this identity goes to configuration state.",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, done: true}",
        example: "canopen-cli lss switch-selective 0x069B 0x1234 1 42",
    },
    Spec {
        name: "lss inquire-identity",
        args: "",
        summary: "Ask vendor, product, revision, serial of the slave in configuration state.",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, identity: {vendor, product, revision, serial}}",
        example: "canopen-cli lss inquire-identity",
    },
    Spec {
        name: "lss inquire-node-id",
        args: "",
        summary: "Ask the node-ID of the slave in configuration state.",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, node_id}",
        example: "canopen-cli lss inquire-node-id",
    },
    Spec {
        name: "lss set-node-id",
        args: "<1..127|255>",
        summary: "Configure the node-ID of the slave in configuration state (active after store + reset, or at once for an unconfigured node on switch to waiting).",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, done: true}",
        example: "canopen-cli lss set-node-id 10",
    },
    Spec {
        name: "lss set-bit-timing",
        args: "<kbit>",
        summary: "Configure the bit rate of the slave in configuration state (standard table).",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, done: true}",
        example: "canopen-cli lss set-bit-timing 500",
    },
    Spec {
        name: "lss activate-bit-timing",
        args: "<delay ms>",
        summary: "Activate the configured bit rate on all slaves after the delay. All nodes change bit rate.",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, done: true}",
        example: "canopen-cli lss activate-bit-timing 100",
    },
    Spec {
        name: "lss store",
        args: "",
        summary: "Store the configured node-ID and bit rate in the slave.",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, done: true}",
        example: "canopen-cli lss store",
    },
    Spec {
        name: "lss fastscan",
        args: "",
        summary: "Find one unconfigured slave by bit scan; it ends in configuration state.",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, found, identity or null}",
        example: "canopen-cli lss fastscan",
    },
    Spec {
        name: "lss identify",
        args: "",
        summary: "Ask all non-configured slaves to answer; counts the answers.",
        sends: true,
        bus: true,
        options: &[LSS_TIMEOUT],
        output: "{service, non_configured}",
        example: "canopen-cli lss identify",
    },
    Spec {
        name: "dcf save",
        args: "<out.dcf>",
        summary: "Read every readable object and write a DCF (EDS plus values, CiA 306).",
        sends: true,
        bus: true,
        options: &[("node-name", Some("text"), "NodeName in [DeviceComissioning]."), ("network-name", Some("text"), "NetworkName in [DeviceComissioning].")],
        output: "{node, file, values, failed, not_stored}",
        example: "canopen-cli dcf save node5.dcf",
    },
    Spec {
        name: "dcf restore",
        args: "<in.dcf>",
        summary: "Write the values of a DCF to the node (PDOs with the CiA 301 sequence). Stops at the first error unless --continue.",
        sends: true,
        bus: true,
        options: &[
            ("dry-run", None, "Read and compare only; write nothing."),
            ("from", Some("index"), "First index (hex)."),
            ("to", Some("index"), "Last index (hex)."),
            ("continue", None, "Go on after a failed write."),
            ("store", None, "Write 0x1010:01 = \"save\" at the end."),
            ("default-values", None, "Also restore objects that have only a DefaultValue."),
        ],
        output: "{node, dry_run, ok, written, would_write, unchanged, skipped, failed, objects: [{index, sub, name, dcf_value, node_value, action}], stored}",
        example: "canopen-cli dcf restore node5.dcf --dry-run",
    },
    Spec {
        name: "dcf compare",
        args: "<in.dcf>",
        summary: "Read the node and list the objects whose value differs from the DCF. Writes nothing.",
        sends: true,
        bus: true,
        options: &[("default-values", None, "Also compare objects that have only a DefaultValue.")],
        output: "{node, compared, equal, differences: [{index, sub, name, dcf_value, node_value, error}]}",
        example: "canopen-cli dcf compare node5.dcf",
    },
    Spec {
        name: "shell",
        args: "",
        summary: "Keep one connection open; read one command per line from stdin (or --script), write one JSON line per result. `exit` ends. The simulator keeps its state for the whole shell.",
        sends: false,
        bus: false,
        options: &[("script", Some("file"), "Read the lines from this file instead of stdin.")],
        output: "JSON Lines: one result object per line ({ok:false, error} for a failed line).",
        example: "canopen-cli shell --script steps.txt",
    },
];

pub fn find(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|s| s.name == name)
}

fn opts_json(o: &[Opt]) -> Value {
    Value::Array(o.iter().map(|(n, v, h)| json!({"name": format!("--{n}"), "value": v, "help": h})).collect())
}

pub fn schema(name: &str, version: &str) -> Value {
    json!({
        "name": name,
        "version": version,
        "description": "Command-line CANopen master for scripts and AI agents. Every result is one JSON object (monitor, watch and shell write JSON Lines).",
        "safety": [
            "The default adapter is the simulator: nothing touches hardware unless --adapter pcan is given.",
            "Nothing is sent on the bus unless the command transmits (\"sends\": true below).",
            "With --listen-only every command with \"sends\": true fails with exit code 2 before the adapter is opened.",
            "monitor and watch are bounded: --duration or --count is required."
        ],
        "syntax": "canopen-cli [global options] <command> [arguments] [options]  (options may stand anywhere; --name value or --name=value)",
        "numbers": "Indexes and CAN identifiers are hex (0x prefix optional); sub-indexes and other numbers decimal or 0x hex.",
        "global_options": opts_json(GLOBAL),
        "commands": COMMANDS.iter().map(|s| json!({
            "name": s.name,
            "args": s.args,
            "summary": s.summary,
            "sends": s.sends,
            "opens_adapter": s.bus,
            "options": opts_json(s.options),
            "output": s.output,
            "example": s.example,
        })).collect::<Vec<_>>(),
        "output": {
            "success": "{\"ok\": true, \"command\": <name>, ...fields of the command}",
            "error": "{\"ok\": false, \"error\": {\"kind\": usage|listen_only|file|eds|adapter|abort|timeout|protocol|device|cancelled, \"message\": text, \"abort_code\": \"0x........\" or null}}"
        },
        "exit_codes": {
            "0": "ok",
            "1": "device or protocol error (SDO abort, timeout, LSS error, a failed step)",
            "2": "usage error (bad command, option or value; a sending command with --listen-only; a file or EDS that cannot be read)",
            "3": "adapter or driver error"
        }
    })
}

/// Usage text of all commands (help without --json).
pub fn usage_text(name: &str, version: &str) -> String {
    let mut s = format!("{name} {version}: CANopen master for scripts and AI agents.\n\nUsage: {name} [global options] <command> [arguments] [options]\n\nGlobal options:\n");
    for (n, v, h) in GLOBAL {
        s.push_str(&format!("  --{n}{}  {h}\n", v.map(|v| format!(" <{v}>")).unwrap_or_default()));
    }
    s.push_str("\nCommands (* = transmits on the bus):\n");
    for c in COMMANDS {
        s.push_str(&format!("  {}{} {}\n      {}\n", if c.sends { "*" } else { " " }, c.name, c.args, c.summary));
    }
    s.push_str(&format!("\nJSON is the default output; --text for tables. `{name} help --json` gives the full schema.\nExit codes: 0 ok, 1 device/protocol error, 2 usage error, 3 adapter error.\n"));
    s
}

pub fn command_text(c: &Spec) -> String {
    let mut s = format!("{} {}\n  {}\n  transmits: {}\n", c.name, c.args, c.summary, if c.sends { "yes" } else { "no" });
    for (n, v, h) in c.options {
        s.push_str(&format!("  --{n}{}  {h}\n", v.map(|v| format!(" <{v}>")).unwrap_or_default()));
    }
    s.push_str(&format!("  output: {}\n  example: {}\n", c.output, c.example));
    s
}
