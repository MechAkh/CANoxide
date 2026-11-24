//! Tests of the command-line tool: the built exe is run on the simulator
//! (no window: CREATE_NO_WINDOW), and its JSON and exit codes are checked.

use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_canoxide-cli");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Run {
    code: i32,
    out: String,
}

impl Run {
    /// The output as one JSON object (the last line for JSON Lines).
    fn json(&self) -> Value {
        let last = self.out.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("");
        serde_json::from_str(last).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.out))
    }

    fn lines(&self) -> Vec<Value> {
        self.out.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}"))).collect()
    }
}

fn command() -> Command {
    let mut c = Command::new(EXE);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = CREATE_NO_WINDOW;
    c.stderr(Stdio::piped()).stdout(Stdio::piped());
    c
}

fn run_with(args: &[&str], stdin: Option<&str>) -> Run {
    let mut c = command();
    c.args(args);
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = c.spawn().expect("start canoxide-cli");
    if let Some(text) = stdin {
        let mut i = child.stdin.take().unwrap();
        i.write_all(text.as_bytes()).unwrap();
    }
    let o = child.wait_with_output().unwrap();
    Run { code: o.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&o.stdout).into_owned() }
}

fn run(args: &[&str]) -> Run {
    run_with(args, None)
}

fn ok(args: &[&str]) -> Value {
    let r = run(args);
    assert_eq!(r.code, 0, "{args:?}: {}", r.out);
    let v = r.json();
    assert_eq!(v["ok"], true, "{args:?}: {v}");
    v
}

fn fails(args: &[&str], code: i32, kind: &str) -> Value {
    let r = run(args);
    assert_eq!(r.code, code, "{args:?}: {}", r.out);
    let v = r.json();
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["error"]["kind"], kind, "{v}");
    assert!(v["error"]["message"].is_string(), "{v}");
    assert!(v["error"].get("abort_code").is_some(), "abort_code key: {v}");
    v
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cli_test_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn cli_version_and_help() {
    let v = ok(&["version"]);
    assert_eq!(v["command"], "version");
    assert_eq!(v["name"], "canoxide-cli");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));

    let r = run(&["help"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("sdo read"), "{}", r.out);

    let s = run(&["help", "--json"]).json();
    let cmds = s["commands"].as_array().unwrap();
    assert!(cmds.len() > 30);
    for c in cmds {
        assert!(c["sends"].is_boolean(), "{c}");
        assert!(c["name"].is_string());
    }
    let find = |n: &str| cmds.iter().find(|c| c["name"] == n).unwrap().clone();
    assert_eq!(find("monitor")["sends"], false);
    assert_eq!(find("nmt")["sends"], true);
    assert_eq!(find("eds info")["sends"], false);
    assert!(s["exit_codes"]["3"].is_string());

    let one = run(&["help", "sdo", "write", "--json"]).json();
    assert_eq!(one["name"], "sdo write");
    let r = run(&["help", "nosuch"]);
    assert_eq!(r.code, 2);
}

#[test]
fn cli_usage_errors_exit_2() {
    let r = run(&[]);
    assert_eq!(r.code, 2);
    fails(&["bogus"], 2, "usage");
    fails(&["sdo"], 2, "usage");
    fails(&["sdo", "read"], 2, "usage");
    fails(&["sdo", "read", "0x1008", "--nope", "1"], 2, "usage");
    fails(&["sdo", "read", "zz"], 2, "usage");
    fails(&["--node", "200", "info"], 2, "usage");
    fails(&["--bitrate", "100k", "info"], 2, "usage");
    fails(&["monitor"], 2, "usage");
    fails(&["send", "0x123", "--period", "10"], 2, "usage");
    fails(&["sdo", "read", "0x1008", "--timeout", "1", "--timeout", "2"], 2, "usage");
    fails(&["eds", "info", "no_such_file.eds"], 2, "file");
}

#[test]
fn cli_listen_only_refuses_sending_commands() {
    for args in [
        &["nmt", "start"][..],
        &["sync"],
        &["send", "0x123", "01"],
        &["sdo", "read", "0x1000"],
        &["sdo", "write", "0x2002", "1"],
        &["scan"],
        &["pdo", "enable", "tpdo1"],
        &["heartbeat", "set", "100"],
        &["lss", "fastscan"],
        &["dcf", "restore", "x.dcf"],
    ] {
        let mut a = vec!["--listen-only"];
        a.extend_from_slice(args);
        let v = fails(&a, 2, "listen_only");
        assert!(v["error"]["message"].as_str().unwrap().contains("nothing was opened"), "{v}");
    }
    // --adapter pcan with --listen-only: refused before the driver is loaded.
    fails(&["--adapter", "pcan", "--listen-only", "nmt", "start"], 2, "listen_only");
    // The same for --adapter canfox (the Sontheim driver is not loaded).
    fails(&["--adapter", "canfox", "--listen-only", "nmt", "start"], 2, "listen_only");
    // A bad net number is a usage error (`version` never opens an adapter).
    let v = fails(&["--adapter", "canfox", "--channel", "usb1", "version"], 2, "usage");
    assert!(v["error"]["message"].as_str().unwrap().contains("net number"), "{v}");
    // A receive-only command runs.
    let v = ok(&["--listen-only", "monitor", "--count", "1"]);
    assert_eq!(v["frames"], 1);
}

#[test]
fn cli_adapters_and_eds() {
    let v = ok(&["adapters"]);
    let names: Vec<_> = v["adapters"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["sim", "pcan", "canfox"]);

    let dir = temp("eds");
    let eds = dir.join("sim.eds");
    std::fs::write(&eds, canoxide::can::sim::BUILTIN_EDS).unwrap();
    let e = eds.to_str().unwrap();
    let v = ok(&["eds", "info", e]);
    assert!(v["objects"].as_u64().unwrap() > 10);
    assert_eq!(v["is_dcf"], false);
    let v = ok(&["eds", "objects", e]);
    let objs = v["objects"].as_array().unwrap();
    let sp = objs.iter().find(|o| o["index"] == "0x2002").unwrap();
    assert_eq!(sp["type"], "INTEGER32");
    let v = ok(&["eds", "pdos", e, "--node", "5"]);
    let p = v["pdos"].as_array().unwrap();
    assert!(p.iter().any(|x| x["name"] == "tpdo1" && x["cob_id"] == "0x185"), "{v}");

    // Not an EDS.
    let bad = dir.join("bad.eds");
    std::fs::write(&bad, "hello").unwrap();
    fails(&["eds", "info", bad.to_str().unwrap()], 2, "eds");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_scan_info_sdo() {
    let v = ok(&["scan", "--to", "3"]);
    assert_eq!(v["found"], 1);
    assert_eq!(v["nodes"][0]["node"], 1);
    assert_eq!(v["nodes"][0]["device_type"], "0x00000191");

    let v = ok(&["--node", "7", "info"]);
    let n = &v["nodes"][0];
    assert_eq!(n["node"], 7);
    assert_eq!(n["vendor_id"], "0x0000069B");
    assert_eq!(n["serial"], "0x12345678");

    let v = ok(&["sdo", "read", "0x1018:1"]);
    assert_eq!(v["value"], 0x69B);
    assert_eq!(v["type"], "UNSIGNED32");
    assert_eq!(v["raw"], "9B 06 00 00");
    let v = ok(&["sdo", "read", "0x2000"]);
    assert_eq!(v["value"], "Hello from the simulator");
    let v = ok(&["sdo", "read", "0x2100", "--mode", "block"]);
    assert_eq!(v["size"], 4096);
    assert_eq!(v["mode"], "block");

    let v = ok(&["sdo", "write", "0x2002", "-5"]);
    assert_eq!(v["data"], "FB FF FF FF");
    // EDS limits are checked before anything is sent.
    fails(&["sdo", "write", "0x2002", "5000"], 2, "usage");
    // Device errors: exit 1 with the abort code.
    let v = fails(&["sdo", "read", "0x5000"], 1, "abort");
    assert_eq!(v["error"]["abort_code"], "0x06020000");
    let v = fails(&["sdo", "write", "0x1000", "1", "--type", "u32"], 1, "abort");
    assert!(v["error"]["abort_code"].is_string());
    // The simulated node takes the ID of --node.
    assert_eq!(ok(&["sdo", "read", "0x1000", "--node", "9"])["node"], 9);
    // Text output.
    let r = run(&["sdo", "read", "0x1017", "--text"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("value: 1000"), "{}", r.out);
}

#[test]
fn cli_sdo_file_transfer() {
    let dir = temp("file");
    let f = dir.join("block.bin");
    let fs = f.to_str().unwrap();
    let v = ok(&["sdo", "read", "0x2100", "--out", fs]);
    assert_eq!(v["size"], 4096);
    assert_eq!(std::fs::metadata(&f).unwrap().len(), 4096);
    let inp = dir.join("in.bin");
    std::fs::write(&inp, [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10]).unwrap();
    let v = ok(&["sdo", "write", "0x2003", "--in", inp.to_str().unwrap()]);
    assert_eq!(v["size"], 10);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_od_dump_nmt_sync_send() {
    let v = ok(&["od", "dump"]);
    assert!(v["read"].as_u64().unwrap() > 20);
    let o = v["objects"].as_array().unwrap();
    assert!(o.iter().any(|x| x["index"] == "0x1008" && x["value"] == "CANoxide simulator"));

    let v = ok(&["nmt", "start"]);
    assert_eq!(v["node"], 1);
    assert_eq!(v["all"], false);
    let v = ok(&["nmt", "reset-node", "--all"]);
    assert_eq!(v["node"], 0);
    fails(&["nmt", "go"], 2, "usage");

    let v = ok(&["sync"]);
    assert_eq!(v["id"], "0x080");
    let v = ok(&["send", "0x123", "01 02 03"]);
    assert_eq!(v["dlc"], 3);
    assert_eq!(v["data"], "01 02 03");
    let v = ok(&["send", "0x1ABCDEF", "--ext", "--period", "10", "--count", "3"]);
    assert_eq!(v["sent"], 3);
    let v = ok(&["send", "0x701", "--rtr", "--dlc", "1"]);
    assert_eq!(v["rtr"], true);
    fails(&["send", "0x800", "01"], 2, "usage");
    fails(&["send", "0x123", "01 02 03 04 05 06 07 08 09"], 2, "usage");
}

#[test]
fn cli_monitor_json_lines() {
    let r = run(&["monitor", "--duration", "1.3"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let l = r.lines();
    let last = l.last().unwrap();
    assert_eq!(last["command"], "monitor");
    assert_eq!(last["ok"], true);
    let frames = &l[..l.len() - 1];
    assert_eq!(last["frames"], frames.len());
    // Boot-up, then one heartbeat.
    assert!(frames.len() >= 2, "{}", r.out);
    assert_eq!(frames[0]["id"], "0x701");
    assert_eq!(frames[0]["class"], "heartbeat");
    assert_eq!(frames[0]["dir"], "Rx");
    for f in frames {
        for k in ["t", "dir", "id", "ext", "rtr", "dlc", "data", "type", "class", "node", "text"] {
            assert!(f.get(k).is_some(), "{k} in {f}");
        }
    }
    let v = ok(&["monitor", "--count", "1", "--class", "heartbeat"]);
    assert_eq!(v["frames"], 1);
    fails(&["monitor", "--count", "1", "--class", "nope"], 2, "usage");
}

#[test]
fn cli_pdo_commands() {
    let v = ok(&["pdo", "read"]);
    assert_eq!(v["complete"], true);
    let p = v["pdos"].as_array().unwrap();
    let t1 = p.iter().find(|x| x["name"] == "tpdo1").unwrap();
    assert_eq!(t1["cob_id"], "0x181");
    assert_eq!(t1["valid"], true);
    let v = ok(&["pdo", "read", "--only", "rpdo1"]);
    assert_eq!(v["pdos"].as_array().unwrap().len(), 1);

    let v = ok(&["pdo", "disable", "tpdo1"]);
    assert_eq!(v["changed"], true);
    assert_eq!(v["steps"]["ok"], true);
    let v = ok(&["pdo", "enable", "all-tpdo"]);
    assert_eq!(v["steps"]["ok"], true);

    let v = ok(&["pdo", "set", "tpdo1", "--event", "50", "--type", "254"]);
    assert_eq!(v["before"]["event_timer_ms"], 200);
    assert_eq!(v["after"]["event_timer_ms"], 50);
    assert_eq!(v["after"]["trans_type"], 254);

    let v = ok(&["pdo", "map", "tpdo1", "0x2001:0:16"]);
    assert_eq!(v["after"]["mapping"][0]["index"], "0x2001");
    assert_eq!(v["after"]["mapping"].as_array().unwrap().len(), 1);

    fails(&["pdo", "set", "tpdo1"], 2, "usage");
    fails(&["pdo", "set", "tpdo9", "--event", "5"], 1, "device");
    fails(&["pdo", "map", "tpdo1", "0x2001"], 2, "usage");
    // Mapping a write-only object into a TPDO: the node refuses; exit 1
    // with the step report.
    let v = fails(&["pdo", "map", "tpdo1", "0x2004:0:8"], 1, "abort");
    assert!(v["detail"]["steps"]["done"].is_array(), "{v}");
}

#[test]
fn cli_pdo_read_saves_to_device_library() {
    let dir = temp("lib");
    let v = ok(&["--data-dir", dir.to_str().unwrap(), "pdo", "read"]);
    assert!(v.get("saved").is_none());
    fails(&["--device", "x", "pdo", "read"], 2, "usage");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_heartbeat_and_watch() {
    let v = ok(&["heartbeat", "set", "500"]);
    assert_eq!(v["nodes"], serde_json::json!([1]));
    assert_eq!(v["steps"]["done"][0]["data"], "F4 01");
    let v = ok(&["heartbeat", "set", "200", "--all"]);
    assert_eq!(v["nodes"], serde_json::json!([1]));
    fails(&["heartbeat", "set", "70000"], 2, "usage");

    let r = run(&["watch", "0x2002:0", "--count", "3", "--period", "50"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let l = r.lines();
    assert_eq!(l.len(), 4);
    assert_eq!(l[0]["value"], -100);
    assert_eq!(l[0]["raw"], "9C FF FF FF");
    assert_eq!(l[3]["samples"], 3);
    assert_eq!(l[3]["source"], "sdo");
    fails(&["watch", "0x2002:0"], 2, "usage");
    fails(&["watch", "0x2002:0", "--count", "1", "--period", "10"], 2, "usage");
    // Not mapped in a TPDO.
    fails(&["watch", "pdo:0x2000:0", "--count", "1"], 1, "device");
}

#[test]
fn cli_lss_commands() {
    // In one process (shell) the simulator keeps its LSS state.
    let r = run_with(&["shell"], Some("lss inquire-node-id\nlss switch-global configuration\nlss inquire-node-id\nlss inquire-identity\nlss switch-global waiting\nlss identify\nlss set-bit-timing 123\nexit\n"));
    let l = r.lines();
    assert_eq!(l.len(), 7, "{}", r.out);
    assert_eq!(l[0]["error"]["kind"], "timeout");
    assert_eq!(l[1]["done"], true);
    assert_eq!(l[2]["node_id"], 1);
    assert_eq!(l[3]["identity"]["vendor"], "0x0000069B");
    assert_eq!(l[3]["identity"]["serial"], "0x12345678");
    assert_eq!(l[4]["command"], "lss switch-global");
    assert_eq!(l[5]["non_configured"], 0);
    assert_eq!(l[6]["error"]["kind"], "usage");
    // The last failed line sets the exit code.
    assert_eq!(r.code, 2);
    fails(&["lss", "inquire-identity"], 1, "timeout");
    fails(&["lss", "set-node-id", "0"], 2, "usage");
    fails(&["lss"], 2, "usage");
}

#[test]
fn cli_dcf_save_compare_restore() {
    let dir = temp("dcf");
    let f = dir.join("node1.dcf");
    let fs = f.to_str().unwrap();
    let v = ok(&["dcf", "save", fs, "--node-name", "bench"]);
    assert!(v["values"].as_u64().unwrap() > 20);
    let text = std::fs::read_to_string(&f).unwrap();
    assert!(text.contains("[DeviceComissioning]"));
    assert!(text.contains("NodeName=bench"));

    let v = ok(&["dcf", "compare", fs]);
    assert_eq!(v["equal"], true);
    let v = ok(&["dcf", "restore", fs, "--dry-run"]);
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["written"], 0);

    // A changed value: compare finds it, restore writes it.
    let changed = text.replace("ParameterValue=0xFFFFFF9C", "ParameterValue=33");
    assert!(changed != text, "the DCF holds 0x2002 = -100");
    std::fs::write(&f, changed).unwrap();
    let v = ok(&["dcf", "compare", fs]);
    assert_eq!(v["equal"], false);
    assert!(v["differences"].as_array().unwrap().iter().any(|d| d["index"] == "0x2002"), "{v}");
    let r = run_with(&["shell"], Some(&format!("dcf restore \"{fs}\"\nsdo read 0x2002\nexit\n")));
    assert_eq!(r.code, 0, "{}", r.out);
    let l = r.lines();
    assert_eq!(l[0]["ok"], true, "{}", l[0]);
    assert!(l[0]["written"].as_u64().unwrap() >= 1);
    assert_eq!(l[1]["value"], 33);

    fails(&["dcf", "restore", fs, "--dry-run", "--store"], 2, "usage");
    fails(&["dcf", "save", fs, "--eds", "nofile.eds"], 2, "eds");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_shell_keeps_state() {
    let script = "# comment\n\nsdo write 0x2002 77\nsdo read 0x2002\nsdo read 0x2002 --node 3\nversion\nexit\nsdo read 0x1000\n";
    let r = run_with(&["shell"], Some(script));
    let l = r.lines();
    assert_eq!(l.len(), 4, "{}", r.out);
    assert_eq!(l[0]["written"], true);
    assert_eq!(l[1]["value"], 77);
    assert_eq!(l[2]["error"]["kind"], "timeout");
    assert_eq!(l[3]["command"], "version");
    assert_eq!(r.code, 1);

    // Global options other than --node, --timeout, --text are fixed.
    let r = run_with(&["shell"], Some("--adapter pcan sdo read 0x1000\nshell\n"));
    let l = r.lines();
    assert_eq!(l[0]["error"]["kind"], "usage");
    assert_eq!(l[1]["error"]["kind"], "usage");
    assert_eq!(r.code, 2);

    // End of input without `exit`; --script.
    let dir = temp("shell");
    let s = dir.join("steps.txt");
    std::fs::write(&s, "nmt start\nwatch pdo:0x2001:0 --count 1\n").unwrap();
    let r = run(&["shell", "--script", s.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}", r.out);
    let l = r.lines();
    assert_eq!(l.last().unwrap()["source"], "pdo");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_pcan_without_driver_is_adapter_error() {
    // Only meaningful on a PC without PCANBasic.dll; with the driver the
    // command would open a channel, so it is not run then.
    let has_driver = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("PCANBasic.dll").exists()).unwrap_or(false);
    if has_driver {
        return;
    }
    fails(&["--adapter", "pcan", "sdo", "read", "0x1000"], 3, "adapter");
}

#[test]
fn cli_replay_of_a_log_file() {
    let dir = temp("replay");
    // A made-up candump log: a heartbeat, TPDO2 of node 1 twice, an SDO.
    let log = dir.join("bus.log");
    std::fs::write(
        &log,
        "(1700000000.000000) can0 701#05\n\
         (1700000000.010000) can0 281#05009CFFFFFF\n\
         (1700000000.020000) can0 581#4B171000E8030000\n\
         (1700000000.030000) can0 281#0600F6FFFFFF\n\
         bad line\n",
    )
    .unwrap();
    let l = log.to_str().unwrap();
    let r = run(&["--adapter", "replay", "--file", l, "monitor"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let v = r.lines();
    assert_eq!(v.len(), 5, "{}", r.out);
    assert_eq!(v[1]["t"], 0.01);
    assert_eq!(v[1]["id"], "0x281");
    assert_eq!(v[1]["data"], "05 00 9C FF FF FF");
    assert_eq!(v[4]["frames"], 4);
    // --duration counts log time; --count stops early.
    let r = run(&["--adapter", "replay", "--file", l, "monitor", "--duration", "0.015"]);
    assert_eq!(r.json()["frames"], 2, "{}", r.out);
    let r = run(&["--adapter", "replay", "--file", l, "--speed", "1", "monitor", "--count", "1", "--class", "tpdo"]);
    assert_eq!(r.json()["frames"], 1, "{}", r.out);
    // watch pdo: with the mapping of the EDS.
    let eds = dir.join("dev.eds");
    std::fs::write(&eds, canoxide::can::sim::BUILTIN_EDS).unwrap();
    let r = run(&["--adapter", "replay", "--file", l, "--eds", eds.to_str().unwrap(), "watch", "pdo:0x2001:0"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let v = r.lines();
    assert_eq!((v[0]["value"].clone(), v[1]["value"].clone()), (serde_json::json!(5), serde_json::json!(6)), "{}", r.out);
    assert_eq!(v[2]["samples"], 2);
    // Nothing is sent: commands that need a live bus are refused.
    fails(&["--adapter", "replay", "--file", l, "sdo", "read", "0x1000"], 2, "usage");
    fails(&["--adapter", "replay", "--file", l, "nmt", "start"], 2, "usage");
    fails(&["--adapter", "replay", "--file", l, "watch", "0x2001:0"], 2, "usage");
    fails(&["--adapter", "replay", "--file", l, "watch", "pdo:0x2001:0"], 2, "usage");
    fails(&["--adapter", "replay", "monitor"], 2, "usage");
    fails(&["--file", l, "monitor", "--count", "1"], 2, "usage");
    fails(&["--adapter", "replay", "--file", dir.join("missing.log").to_str().unwrap(), "monitor"], 2, "file");
    let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn cli_check_against_the_simulator() {
    let dir = temp("check");
    let junit = dir.join("check.xml");
    let v = ok(&["check", "--junit", junit.to_str().unwrap()]);
    assert_eq!(v["passed"], true, "{v}");
    assert_eq!(v["failed"], 0, "{v}");
    assert_eq!(v["eds_file"], "sim_builtin.eds");
    assert!(v["checked"].as_u64().unwrap() > 20, "{v}");
    assert!(v["summary"].as_str().unwrap().contains("0 failed"), "{v}");
    let x = std::fs::read_to_string(&junit).unwrap();
    assert!(x.starts_with("<?xml") && x.contains("<testsuite") && x.contains("</testsuite>"), "{x}");
    // --all lists the ok entries too; a range limits the entries.
    let all = ok(&["check", "--all", "--from", "1000", "--to", "1018"]);
    let e = all["entries"].as_array().unwrap();
    assert!(!e.is_empty() && e.iter().all(|x| x["index"].as_str().unwrap() <= "0x1018"), "{all}");
    assert!(e.iter().any(|x| x["kind"] == "ok"), "{all}");
    // --text prints the summary.
    let r = run(&["check", "--text"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("summary:"), "{}", r.out);
    fails(&["check", "--from", "2000", "--to", "1000"], 2, "usage");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_emcy_history_and_clear() {
    let v = ok(&["emcy", "history", "--node", "4"]);
    assert_eq!(v["node"], 4);
    assert_eq!(v["count"], 2, "{v}");
    assert_eq!(v["entries"][0]["code"], "0x8130", "{v}");
    assert_eq!(v["entries"][0]["text"], "Life guard or heartbeat error", "{v}");
    assert_eq!(v["entries"][0]["info"], "0x0011", "{v}");
    assert_eq!(v["entries"][1]["code"], "0x5000", "{v}");
    // In one process the clear empties the history.
    let r = run_with(&["shell"], Some("emcy clear\nemcy history\nexit\n"));
    assert_eq!(r.code, 0, "{}", r.out);
    let l = r.lines();
    assert_eq!(l[0]["cleared"], true, "{}", l[0]);
    assert_eq!(l[1]["count"], 0, "{}", l[1]);
}

#[test]
fn cli_store_and_restore_defaults() {
    let v = ok(&["store"]);
    assert_eq!((v["index"].clone(), v["sub"].clone(), v["stored"].clone()), (serde_json::json!("0x1010"), serde_json::json!(1), serde_json::json!(true)), "{v}");
    assert_eq!(v["name"], "All parameters");
    let v = ok(&["restore-defaults", "--sub", "2"]);
    assert_eq!(v["index"], "0x1011");
    assert_eq!(v["restored"], true);
    assert_eq!(v["name"], "Communication parameters");
    assert!(v["note"].as_str().unwrap().contains("reset node"), "{v}");
    // The simulator has subs 1..3 only.
    fails(&["store", "--sub", "9"], 1, "abort");
    fails(&["store", "--sub", "0"], 2, "usage");
}

#[test]
fn cli_new_writing_commands_refused_listen_only() {
    for args in [&["check"][..], &["emcy", "history"], &["emcy", "clear"], &["store"], &["restore-defaults"]] {
        let mut a = vec!["--listen-only"];
        a.extend_from_slice(args);
        let v = fails(&a, 2, "listen_only");
        assert!(v["error"]["message"].as_str().unwrap().contains("nothing was opened"), "{v}");
    }
}
