//! The commands. Each returns the fields of its JSON result or a
//! `CliError`; `monitor` and `watch` also write one JSON line per sample.

use super::{parse_node, spec, Args, CliError, CmdResult, Ctx, CLI_NAME};
use canoxide::can::pcan::PcanAdapter;
use canoxide::can::sontheim::SontheimAdapter;
use canoxide::can::{CanAdapter, CanFrame};
use canoxide::canopen::decode::{DecodeContext, MsgClass, SdoBlockTracker};
use canoxide::canopen::lss::{bit_timing_index, LssCommand, LssError, LssIdentity, LssResult, DEFAULT_LSS_TIMEOUT_MS};
use canoxide::canopen::nmt::{nmt_frame, sync_frame, NmtCommand};
use canoxide::canopen::pdo::{
    configs_from_od, extract_bits, parse_map_entry, parse_pdo_name, plan_change, plan_enable, plan_enable_all, MapEntry, NodePdoConfig, PdoChange,
    PdoConfig, PdoKind, PdoReadOptions, PdoReadTask, DEFAULT_PDO_SCAN_STOP, MAX_PDOS,
};
use canoxide::canopen::sdo::{SdoError, SdoRequest, StartOptions, TransferMode, MAX_BLOCK_SIZE};
use canoxide::canopen::task::{ReadsTask, SdoStep, SdoTask, StepsReport, StepsTask, TaskResult};
use canoxide::canopen::types::{self, format_value, hex_bytes, le_u64, parse_int, parse_text, DataType, DisplayMode, Value as TValue};
use canoxide::dcf::{readable_objects, DcfCommissioning, DcfCompareTask, DcfRestoreTask, ObjectAction, RestoreOptions};
use canoxide::eds::{self, ObjectDictionary};
use canoxide::netinfo::{heartbeat_steps, NodeInfo, NodeInfoTask};
use canoxide::session::{download_from_file, upload_to_file, Ctl, FileTransferError, SessionError};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---- helpers ----------------------------------------------------------

pub fn hex16(v: u16) -> String {
    format!("0x{v:04X}")
}

pub fn hex32(v: u32) -> String {
    format!("0x{v:08X}")
}

fn cob(v: u32) -> String {
    format!("0x{v:03X}")
}

fn arg<'a>(pos: &'a [String], i: usize, what: &str) -> Result<&'a str, CliError> {
    pos.get(i).map(|s| s.as_str()).ok_or_else(|| CliError::usage(format!("missing argument: {what}")))
}

fn max_args(pos: &[String], n: usize) -> Result<(), CliError> {
    if pos.len() > n {
        return Err(CliError::usage(format!("unexpected argument '{}'", pos[n])));
    }
    Ok(())
}

fn parse_hex_u32(s: &str, what: &str) -> Result<u32, CliError> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(t, 16).map_err(|_| CliError::usage(format!("{what}: '{s}' is not a hex number")))
}

/// An index (hex, `0x` optional).
pub fn parse_index(s: &str) -> Result<u16, CliError> {
    let v = parse_hex_u32(s, "index")?;
    u16::try_from(v).map_err(|_| CliError::usage(format!("index '{s}' above 0xFFFF")))
}

/// `index[:sub]`: index hex, sub decimal or `0x` hex.
pub fn parse_obj(s: &str) -> Result<(u16, u8), CliError> {
    let (i, sub) = match s.split_once(':') {
        Some((i, sub)) => (i, Some(sub)),
        None => (s, None),
    };
    let index = parse_index(i)?;
    let sub = match sub {
        None => 0,
        Some(t) => match parse_int(t, false) {
            Some(x) if (0..=255).contains(&x) => x as u8,
            _ => return Err(CliError::usage(format!("sub-index '{t}' is not 0..255"))),
        },
    };
    Ok((index, sub))
}

pub fn parse_type(s: &str) -> Result<DataType, CliError> {
    use DataType::*;
    let l = s.trim().to_ascii_lowercase();
    let t = match l.as_str() {
        "bool" | "boolean" => Some(Boolean),
        "i8" | "int8" => Some(Integer8),
        "i16" | "int16" => Some(Integer16),
        "i24" | "int24" => Some(Integer24),
        "i32" | "int32" => Some(Integer32),
        "i40" => Some(Integer40),
        "i48" => Some(Integer48),
        "i56" => Some(Integer56),
        "i64" | "int64" => Some(Integer64),
        "u8" | "uint8" => Some(Unsigned8),
        "u16" | "uint16" => Some(Unsigned16),
        "u24" | "uint24" => Some(Unsigned24),
        "u32" | "uint32" => Some(Unsigned32),
        "u40" => Some(Unsigned40),
        "u48" => Some(Unsigned48),
        "u56" => Some(Unsigned56),
        "u64" | "uint64" => Some(Unsigned64),
        "f32" | "real32" | "float" => Some(Real32),
        "f64" | "real64" | "double" => Some(Real64),
        "string" | "str" | "visible_string" => Some(VisibleString),
        "unicode" | "unicode_string" => Some(UnicodeString),
        "octets" | "octet_string" => Some(OctetString),
        "domain" | "bytes" => Some(Domain),
        _ => None,
    };
    t.or_else(|| DataType::ALL.iter().copied().find(|d| d.name().eq_ignore_ascii_case(&l)))
        .or_else(|| parse_int(&l, false).and_then(|c| u16::try_from(c).ok()).and_then(DataType::from_code))
        .ok_or_else(|| CliError::usage(format!("--type: unknown data type '{s}'")))
}

fn parse_mode(a: &Args) -> Result<TransferMode, CliError> {
    match a.get("mode") {
        None => Ok(TransferMode::Auto),
        Some(m) => TransferMode::parse(m).ok_or_else(|| CliError::usage(format!("--mode: '{m}' is not auto, segmented or block"))),
    }
}

pub fn sess_err(e: SessionError) -> CliError {
    match e {
        SessionError::ListenOnly(w) => CliError::new("listen_only", format!("{w}: listen-only mode, nothing sent")),
        SessionError::Adapter(m) => CliError::new("adapter", m),
    }
}

pub fn sdo_err(e: &SdoError) -> CliError {
    let kind = match e {
        SdoError::ServerAbort(_) => "abort",
        SdoError::ClientAbort(_) => "protocol",
        SdoError::Timeout => "timeout",
        SdoError::Cancelled => "cancelled",
        SdoError::InvalidNode => "usage",
        SdoError::Busy => "device",
    };
    CliError { kind, message: e.to_string(), abort_code: e.abort_code(), detail: None }
}

fn ft_err(e: FileTransferError) -> CliError {
    match e {
        FileTransferError::Session(s) => sess_err(s),
        FileTransferError::Sdo(s) => sdo_err(&s),
        FileTransferError::Io(m) => CliError::new("file", m),
    }
}

fn lss_err(e: LssError) -> CliError {
    let kind = match &e {
        LssError::Timeout => "timeout",
        LssError::Refused { .. } | LssError::Busy => "device",
        LssError::Protocol(_) => "protocol",
        LssError::Cancelled => "cancelled",
        LssError::InvalidArgument(_) => "usage",
    };
    CliError::new(kind, e.to_string())
}

fn load_file_od(path: &str) -> Result<ObjectDictionary, CliError> {
    let od = eds::load(Path::new(path)).map_err(|e| CliError::new("file", format!("{path}: {e}")))?;
    if od.objects.is_empty() {
        return Err(CliError::new("eds", format!("{path}: no objects found (not an EDS/DCF?)")));
    }
    Ok(od)
}

/// A value as JSON: type, size, the typed value, hex, decimal and string
/// forms and the raw bytes.
fn value_json(dt: Option<DataType>, b: &[u8]) -> Map<String, Value> {
    let typed = match dt.and_then(|t| types::decode(t, b).ok()) {
        Some(TValue::Bool(x)) => json!(x),
        Some(TValue::Int(i)) => json!(i),
        Some(TValue::UInt(u)) => json!(u),
        Some(TValue::F32(f)) => json!(f as f64),
        Some(TValue::F64(f)) => json!(f),
        Some(TValue::Str(s)) => json!(s),
        Some(TValue::Bytes(x)) => json!(hex_bytes(&x)),
        None => json!(hex_bytes(b)),
    };
    let mut m = Map::new();
    m.insert("type".into(), json!(dt.map(|t| t.name())));
    m.insert("size".into(), json!(b.len()));
    m.insert("value".into(), typed);
    m.insert("hex".into(), json!(format_value(dt, b, DisplayMode::Hex)));
    m.insert("decimal".into(), json!(format_value(dt, b, DisplayMode::Decimal)));
    m.insert("string".into(), json!(format_value(dt, b, DisplayMode::String)));
    m.insert("raw".into(), json!(hex_bytes(b)));
    m
}

fn obj_head(node: u8, index: u16, sub: u8, od: Option<&ObjectDictionary>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("node".into(), json!(node));
    m.insert("index".into(), json!(hex16(index)));
    m.insert("sub".into(), json!(sub));
    m.insert("name".into(), json!(od.and_then(|o| o.name_of(index, sub))));
    m
}

fn entry_type(od: Option<&ObjectDictionary>, index: u16, sub: u8) -> Option<DataType> {
    od.and_then(|o| o.entry(index, sub)).and_then(|e| e.dtype())
}

fn run_task(ctx: &mut Ctx, task: Box<dyn SdoTask>) -> Result<TaskResult, CliError> {
    let s = ctx.session()?;
    s.run_task(task, &mut Ctl::default()).map_err(sess_err)
}

fn internal() -> CliError {
    CliError::new("device", "internal error: unexpected task result")
}

fn pdo_json(c: &PdoConfig, od: Option<&ObjectDictionary>) -> Value {
    json!({
        "name": c.short_name(),
        "kind": c.kind.label(),
        "number": c.number,
        "cob_id": cob(c.cob_id),
        "valid": c.valid,
        "extended": c.extended,
        "rtr_allowed": c.rtr_allowed,
        "trans_type": c.trans_type,
        "inhibit_100us": c.inhibit_100us,
        "event_timer_ms": c.event_timer_ms,
        "sync_start": c.sync_start,
        "mapping": c.mapping.iter().map(|m| json!({
            "index": hex16(m.index),
            "sub": m.sub,
            "bits": m.bits,
            "name": od.and_then(|o| o.name_of(m.index, m.sub)),
        })).collect::<Vec<_>>(),
    })
}

fn node_pdo_json(cfg: &NodePdoConfig, od: Option<&ObjectDictionary>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("node".into(), json!(cfg.node));
    m.insert("read_time".into(), json!(cfg.read_time));
    m.insert("read_time_text".into(), json!(cfg.read_time_text()));
    m.insert("complete".into(), json!(cfg.complete));
    m.insert("errors".into(), json!(cfg.errors));
    m.insert("pdos".into(), Value::Array(cfg.pdos.iter().map(|p| pdo_json(p, od)).collect()));
    m
}

fn step_json(s: &SdoStep) -> Value {
    json!({"what": s.what, "node": s.node, "index": hex16(s.index), "sub": s.sub, "data": hex_bytes(&s.data)})
}

fn steps_json(r: &StepsReport) -> Value {
    json!({
        "ok": r.ok,
        "cancelled": r.cancelled,
        "done": r.done.iter().map(|d| {
            let mut v = step_json(&d.step);
            v["ok"] = json!(d.ok);
            v["error"] = json!(d.error);
            v["abort_code"] = json!(d.abort_code.map(hex32));
            v
        }).collect::<Vec<_>>(),
        "not_run": r.not_run.iter().map(step_json).collect::<Vec<_>>(),
    })
}

/// Ok(out), or a device error that carries `out` as the detail.
fn finish_steps(r: &StepsReport, out: Map<String, Value>) -> CmdResult {
    if r.ok {
        return Ok(Value::Object(out));
    }
    let (msg, code) = match r.first_error() {
        Some(e) => (format!("{}: {}", e.step.what, e.error.clone().unwrap_or_default()), e.abort_code),
        None => ("cancelled".to_string(), None),
    };
    let kind = if code.is_some() { "abort" } else { "device" };
    Err(CliError { kind, message: msg, abort_code: code, detail: Some(Value::Object(out)) })
}

fn run_steps(ctx: &mut Ctx, label: &str, steps: Vec<SdoStep>, stop_on_error: bool) -> Result<StepsReport, CliError> {
    match run_task(ctx, Box::new(StepsTask::new(label, steps, stop_on_error)))? {
        TaskResult::Steps(r) => Ok(r),
        _ => Err(internal()),
    }
}

fn read_pdos(ctx: &mut Ctx, only: Option<(PdoKind, u16)>, stop: u16) -> Result<NodePdoConfig, CliError> {
    let node = ctx.node;
    let opts = PdoReadOptions { stop_after_missing: stop, max: MAX_PDOS, only };
    match run_task(ctx, Box::new(PdoReadTask::new(node, opts)))? {
        TaskResult::PdoConfig(c) => Ok(c),
        _ => Err(internal()),
    }
}

fn pdo_name(s: &str) -> Result<(PdoKind, u16), CliError> {
    parse_pdo_name(s).ok_or_else(|| CliError::usage(format!("'{s}' is not a PDO name (rpdo1..rpdo512, tpdo1..tpdo512)")))
}

/// Read one PDO from the node; an error when the node does not have it.
fn read_one_pdo(ctx: &mut Ctx, kind: PdoKind, n: u16) -> Result<PdoConfig, CliError> {
    let cfg = read_pdos(ctx, Some((kind, n)), DEFAULT_PDO_SCAN_STOP)?;
    cfg.get(kind, n).cloned().ok_or_else(|| {
        let e = if cfg.errors.is_empty() { String::new() } else { format!(" ({})", cfg.errors.join("; ")) };
        CliError::new("device", format!("{}{n} does not exist on node {}{e}", kind.label(), ctx.node))
    })
}

/// Nodes of `--nodes`, `--all` (a scan) or `--node`.
fn target_nodes(ctx: &mut Ctx, a: &Args) -> Result<Vec<u8>, CliError> {
    if a.flag("all") {
        if a.get("nodes").is_some() {
            return Err(CliError::usage("give --nodes or --all, not both"));
        }
        return Ok(scan_nodes(ctx, 1, 127, 100)?.into_iter().map(|(n, _)| n).collect());
    }
    match a.get("nodes") {
        Some(list) => list.split(',').map(|n| parse_node(n.trim(), "--nodes")).collect(),
        None => Ok(vec![ctx.node]),
    }
}

/// A node that answered a scan, with its 0x1000 read.
type ScanHit = (u8, Result<Vec<u8>, SdoError>);

/// Nodes that answer (data or abort) a read of 0x1000.
fn scan_nodes(ctx: &mut Ctx, from: u8, to: u8, wait_ms: u64) -> Result<Vec<ScanHit>, CliError> {
    let s = ctx.session()?;
    let mut found = Vec::new();
    for n in from..=to {
        let opts = StartOptions { mode: TransferMode::Auto, timeout_ms: Some(wait_ms), abort_on_timeout: false };
        let o = s.sdo(n, SdoRequest::Upload { index: 0x1000, sub: 0 }, opts).map_err(sess_err)?;
        match o.result {
            Err(SdoError::Timeout) => {}
            r => found.push((n, r)),
        }
    }
    Ok(found)
}

// ---- dispatch -----------------------------------------------------------

/// `help` at the top level: usage text, or the schema with --json.
pub fn help(rest: &[String], a: &Args, out: &mut dyn Write) -> i32 {
    match help_value(rest, a) {
        Ok(Value::String(t)) => {
            let _ = out.write_all(t.as_bytes());
            0
        }
        Ok(v) => {
            let _ = writeln!(out, "{v}");
            0
        }
        Err(e) => {
            let code = e.exit_code();
            let _ = writeln!(out, "{}", e.to_json());
            code
        }
    }
}

fn help_value(rest: &[String], a: &Args) -> Result<Value, CliError> {
    let version = canoxide::VERSION;
    if rest.is_empty() {
        return Ok(if a.flag("json") { spec::schema(CLI_NAME, version) } else { Value::String(spec::usage_text(CLI_NAME, version)) });
    }
    let name = rest.join(" ").to_ascii_lowercase();
    let c = spec::find(&name).ok_or_else(|| CliError::usage(format!("help: unknown command '{name}'")))?;
    if a.flag("json") {
        let all = spec::schema(CLI_NAME, version);
        let one = all["commands"].as_array().and_then(|v| v.iter().find(|x| x["name"] == c.name)).cloned().unwrap_or(Value::Null);
        return Ok(one);
    }
    Ok(Value::String(spec::command_text(c)))
}

pub fn dispatch(ctx: &mut Ctx, name: &str, pos: &[String], a: &Args, out: &mut dyn Write) -> CmdResult {
    match name {
        "help" => {
            let v = help_value(pos, a)?;
            Ok(match v {
                Value::String(t) => json!({ "text": t }),
                v => json!({ "schema": v }),
            })
        }
        "version" => {
            max_args(pos, 0)?;
            Ok(json!({"name": CLI_NAME, "version": canoxide::VERSION}))
        }
        "adapters" => adapters(ctx, pos),
        "eds info" => eds_info(pos),
        "eds objects" => eds_objects(pos),
        "eds pdos" => eds_pdos(ctx, pos),
        "scan" => scan(ctx, pos, a),
        "info" => info(ctx, pos, a),
        "sdo read" => sdo_read(ctx, pos, a),
        "sdo write" => sdo_write(ctx, pos, a),
        "od dump" => od_dump(ctx, pos),
        "nmt" => nmt(ctx, pos, a),
        "sync" => {
            max_args(pos, 0)?;
            ctx.session()?.send(sync_frame()).map_err(sess_err)?;
            Ok(json!({"sent": 1, "id": cob(0x080)}))
        }
        "send" => send(ctx, pos, a),
        "monitor" => monitor(ctx, pos, a, out),
        "pdo read" => pdo_read(ctx, pos, a),
        "pdo enable" => pdo_enable(ctx, pos, true),
        "pdo disable" => pdo_enable(ctx, pos, false),
        "pdo set" => pdo_set(ctx, pos, a),
        "pdo map" => pdo_map(ctx, pos),
        "heartbeat set" => heartbeat_set(ctx, pos, a),
        "watch" => watch(ctx, pos, a, out),
        n if n.starts_with("lss ") => lss(ctx, &n[4..], pos, a),
        "dcf save" => dcf_save(ctx, pos, a),
        "dcf restore" => dcf_restore(ctx, pos, a),
        "dcf compare" => dcf_compare(ctx, pos, a),
        other => Err(CliError::usage(format!("'{other}' cannot run here"))),
    }
}

// ---- commands ---------------------------------------------------------

fn adapters(ctx: &mut Ctx, pos: &[String]) -> CmdResult {
    max_args(pos, 0)?;
    let mut list = vec![json!({
        "name": "sim",
        "description": "Built-in simulated CANopen node (the default). Its state lives as long as the process; use `shell` for multi-step work.",
    })];
    let mut pcan = json!({
        "name": "pcan",
        "description": "PEAK PCAN-USB through PCANBasic.dll. Channels are listed with --adapter pcan (the driver is asked; no channel is opened).",
    });
    let canfox_asked = ctx.g.adapter == "canfox";
    if !ctx.is_sim() && !canfox_asked {
        let mut a = PcanAdapter::load().map_err(|e| CliError::new("adapter", format!("PCAN driver: {e}")))?;
        let ch = a.list_channels().map_err(|e| CliError::new("adapter", format!("PCAN driver: {e}")))?;
        pcan["channels"] = Value::Array(
            ch.iter()
                .map(|c| {
                    let usb = canoxide::can::pcan::PCAN_USB_CHANNELS.iter().position(|&h| h as u32 == c.handle).map(|i| format!("usb{}", i + 1));
                    json!({"name": usb, "handle": format!("0x{:02X}", c.handle), "label": c.name, "available": c.available})
                })
                .collect(),
        );
    }
    list.push(pcan);
    let mut canfox = json!({
        "name": "canfox",
        "description": "ifm CANfox through the Sontheim MT-API (SIECA164.DLL). --channel is the net number (default 105). Nets are listed with --adapter canfox (the driver is asked; no net is opened).",
    });
    if canfox_asked {
        let mut a = SontheimAdapter::load().map_err(|e| CliError::new("adapter", format!("CANfox driver: {e}")))?;
        let ch = a.list_channels().map_err(|e| CliError::new("adapter", format!("CANfox driver: {e}")))?;
        canfox["channels"] = Value::Array(ch.iter().map(|c| json!({"net": c.handle, "label": c.name, "available": c.available})).collect());
    }
    list.push(canfox);
    Ok(json!({ "adapters": list }))
}

fn kv(pairs: &[(String, String)]) -> Value {
    Value::Object(pairs.iter().map(|(k, v)| (k.clone(), json!(v))).collect())
}

fn eds_info(pos: &[String]) -> CmdResult {
    max_args(pos, 1)?;
    let path = arg(pos, 0, "<file>")?;
    let od = load_file_od(path)?;
    Ok(json!({
        "file": path,
        "objects": od.objects.len(),
        "entries": od.entry_count(),
        "is_dcf": od.is_dcf,
        "node_id": od.node_id,
        "baudrate_kbit": od.baudrate_kbit,
        "file_info": kv(&od.file_info),
        "device_info": kv(&od.device_info),
        "warnings": od.warnings,
    }))
}

fn eds_objects(pos: &[String]) -> CmdResult {
    max_args(pos, 1)?;
    let path = arg(pos, 0, "<file>")?;
    let od = load_file_od(path)?;
    let mut list = Vec::new();
    for o in od.objects.values() {
        for e in o.subs.values() {
            list.push(json!({
                "index": hex16(o.index),
                "sub": e.sub,
                "name": od.name_of(o.index, e.sub),
                "type": e.dtype().map(|t| t.name().to_string()).unwrap_or_else(|| types::type_name(e.data_type)),
                "access": e.access.label(),
                "default": e.default,
                "value": e.param_value,
                "low": e.low,
                "high": e.high,
                "pdo_mapping": e.pdo_mapping,
            }));
        }
    }
    Ok(json!({"file": path, "count": list.len(), "objects": list}))
}

fn eds_pdos(ctx: &mut Ctx, pos: &[String]) -> CmdResult {
    max_args(pos, 1)?;
    let path = arg(pos, 0, "<file>")?;
    let od = load_file_od(path)?;
    let node = od.node_id.filter(|_| od.is_dcf).unwrap_or(ctx.node);
    let pdos: Vec<Value> = configs_from_od(&od, node).iter().map(|p| pdo_json(p, Some(&od))).collect();
    Ok(json!({"file": path, "node": node, "pdos": pdos}))
}

fn scan(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 0)?;
    let from = a.get("from").map(|s| parse_node(s, "--from")).transpose()?.unwrap_or(1);
    let to = a.get("to").map(|s| parse_node(s, "--to")).transpose()?.unwrap_or(127);
    if from > to {
        return Err(CliError::usage("--from is above --to"));
    }
    let wait = a.num::<u64>("scan-timeout")?.unwrap_or(100).clamp(10, 10_000);
    let found = scan_nodes(ctx, from, to, wait)?;
    let mut nodes = Vec::new();
    let mut errors = Vec::new();
    for (n, r) in found {
        match r {
            Ok(d) => nodes.push(json!({"node": n, "device_type": hex32(le_u64(&d) as u32)})),
            Err(e) if e.abort_code().is_some() => nodes.push(json!({"node": n, "device_type": null, "abort_code": e.abort_code().map(hex32)})),
            Err(e) => errors.push(json!({"node": n, "error": e.to_string()})),
        }
    }
    Ok(json!({"from": from, "to": to, "found": nodes.len(), "nodes": nodes, "errors": errors}))
}

fn node_info_json(i: &NodeInfo) -> Value {
    let h = |v: Option<u32>| v.map(hex32);
    json!({
        "node": i.node,
        "state": i.state,
        "device_type": h(i.device_type),
        "error_register": i.error_register.map(|r| format!("0x{r:02X}")),
        "vendor_id": h(i.vendor_id),
        "product_code": h(i.product_code),
        "revision": h(i.revision),
        "serial": h(i.serial),
        "device_name": i.device_name,
        "last_emcy": i.last_emcy.as_ref().map(|e| json!({"code": format!("0x{:04X}", e.code), "register": format!("0x{:02X}", e.register), "text": e.text, "data": hex_bytes(&e.data)})),
        "read_errors": i.read_errors,
    })
}

fn info(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 0)?;
    let nodes = target_nodes(ctx, a)?;
    let infos = match run_task(ctx, Box::new(NodeInfoTask::new(&nodes)))? {
        TaskResult::NodeInfo(v) => v,
        _ => return Err(internal()),
    };
    if let Some(s) = ctx.session.as_mut() {
        for f in s.take_frames() {
            ctx.overview.on_frame(&f);
        }
    }
    let mut list = Vec::new();
    for i in &infos {
        ctx.overview.merge(i);
        if let Some(n) = ctx.overview.get(i.node) {
            list.push(node_info_json(n));
        }
    }
    Ok(json!({"nodes": list}))
}

fn sdo_read(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 1)?;
    let (index, sub) = parse_obj(arg(pos, 0, "<index>[:sub]")?)?;
    let od = ctx.dict();
    let dt = match a.get("type") {
        Some(t) => Some(parse_type(t)?),
        None => entry_type(od.as_deref(), index, sub),
    };
    let mode = parse_mode(a)?;
    let bs = a.num::<u8>("block-size")?;
    if bs.is_some_and(|b| b == 0 || b > MAX_BLOCK_SIZE) {
        return Err(CliError::usage("--block-size must be 1..127"));
    }
    let node = ctx.node;
    let mut m = obj_head(node, index, sub, od.as_deref());
    let s = ctx.session()?;
    if let Some(b) = bs {
        s.sdo.block_size = b;
    }
    if let Some(p) = a.get("out") {
        let n = upload_to_file(s, node, index, sub, StartOptions::mode(mode), Path::new(p), &mut Ctl::default()).map_err(ft_err)?;
        m.insert("mode".into(), json!(mode.label()));
        m.insert("file".into(), json!(p));
        m.insert("size".into(), json!(n));
        return Ok(Value::Object(m));
    }
    let o = s.sdo(node, SdoRequest::Upload { index, sub }, StartOptions::mode(mode)).map_err(sess_err)?;
    let data = o.result.map_err(|e| sdo_err(&e))?;
    m.insert("mode".into(), json!(mode.label()));
    m.extend(value_json(dt, &data));
    Ok(Value::Object(m))
}

fn sdo_write(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    let (index, sub) = parse_obj(arg(pos, 0, "<index>[:sub]")?)?;
    let od = ctx.dict();
    let entry = od.as_deref().and_then(|o| o.entry(index, sub)).cloned();
    let dt = match a.get("type") {
        Some(t) => Some(parse_type(t)?),
        None => entry.as_ref().and_then(|e| e.dtype()),
    };
    let mode = parse_mode(a)?;
    let bs = a.num::<u8>("block-size")?;
    if bs.is_some_and(|b| b == 0 || b > MAX_BLOCK_SIZE) {
        return Err(CliError::usage("--block-size must be 1..127"));
    }
    let node = ctx.node;
    let mut m = obj_head(node, index, sub, od.as_deref());
    m.insert("mode".into(), json!(mode.label()));
    if let Some(p) = a.get("in") {
        max_args(pos, 1)?;
        let s = ctx.session()?;
        if let Some(b) = bs {
            s.sdo.block_size = b;
        }
        let n = download_from_file(s, node, index, sub, StartOptions::mode(mode), Path::new(p), &mut Ctl::default()).map_err(ft_err)?;
        m.insert("file".into(), json!(p));
        m.insert("size".into(), json!(n));
        m.insert("written".into(), json!(true));
        return Ok(Value::Object(m));
    }
    if pos.len() < 2 {
        return Err(CliError::usage("missing argument: <value> (or --in <file>)"));
    }
    let text = pos[1..].join(" ");
    let Some(dt) = dt else {
        return Err(CliError::usage(format!("no data type for {}:{sub}: give --type or an --eds that has the object", hex16(index))));
    };
    let data = parse_text(dt, &text, false).map_err(|e| CliError::usage(format!("value '{text}': {e}")))?;
    if let Some(e) = &entry {
        if a.get("type").is_none() {
            types::check_limits(dt, &data, e.low.as_deref(), e.high.as_deref()).map_err(|x| CliError::usage(format!("value '{text}': {x} (EDS limits)")))?;
        }
    }
    let s = ctx.session()?;
    if let Some(b) = bs {
        s.sdo.block_size = b;
    }
    let o = s.sdo(node, SdoRequest::Download { index, sub, data: data.clone() }, StartOptions::mode(mode)).map_err(sess_err)?;
    o.result.map_err(|e| sdo_err(&e))?;
    m.insert("type".into(), json!(dt.name()));
    m.insert("size".into(), json!(data.len()));
    m.insert("data".into(), json!(hex_bytes(&data)));
    m.insert("written".into(), json!(true));
    Ok(Value::Object(m))
}

fn od_dump(ctx: &mut Ctx, pos: &[String]) -> CmdResult {
    max_args(pos, 0)?;
    let Some(od) = ctx.dict() else {
        return Err(CliError::usage("od dump needs --eds (the objects to read)"));
    };
    let node = ctx.node;
    let rep = match run_task(ctx, Box::new(ReadsTask::new("od dump", node, readable_objects(&od))))? {
        TaskResult::Reads(r) => r,
        _ => return Err(internal()),
    };
    let mut objects = Vec::new();
    let mut failed = 0;
    for r in &rep.reads {
        let dt = entry_type(Some(&od), r.index, r.sub);
        let mut m = obj_head(node, r.index, r.sub, Some(&od));
        m.remove("node");
        match &r.data {
            Some(d) => {
                let v = value_json(dt, d);
                m.insert("type".into(), v["type"].clone());
                m.insert("value".into(), v["value"].clone());
                m.insert("hex".into(), v["hex"].clone());
            }
            None => {
                failed += 1;
                m.insert("error".into(), json!(r.error));
                m.insert("abort_code".into(), json!(r.abort_code.map(hex32)));
            }
        }
        objects.push(Value::Object(m));
    }
    Ok(json!({"node": node, "read": rep.reads.len() - failed, "failed": failed, "cancelled": rep.cancelled, "objects": objects}))
}

fn nmt(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 1)?;
    let w = arg(pos, 0, "start|stop|preop|reset-node|reset-comm")?.to_ascii_lowercase();
    let cmd = match w.as_str() {
        "start" => NmtCommand::Start,
        "stop" => NmtCommand::Stop,
        "preop" | "pre-operational" => NmtCommand::PreOperational,
        "reset-node" => NmtCommand::ResetNode,
        "reset-comm" | "reset-communication" => NmtCommand::ResetCommunication,
        _ => return Err(CliError::usage(format!("nmt: '{w}' is not start, stop, preop, reset-node or reset-comm"))),
    };
    if a.flag("all") && a.get("node").is_some() {
        return Err(CliError::usage("give --node or --all, not both"));
    }
    let node = if a.flag("all") { 0 } else { ctx.node };
    let s = ctx.session()?;
    s.send(nmt_frame(cmd, node)).map_err(sess_err)?;
    // Let a simulated node act on it before the next command.
    s.listen(Duration::from_millis(20), |_| true).map_err(sess_err)?;
    Ok(json!({"service": w, "label": cmd.label(), "node": node, "all": node == 0}))
}

fn send(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    let id = parse_hex_u32(arg(pos, 0, "<id>")?, "id")?;
    let ext = a.flag("ext");
    let rtr = a.flag("rtr");
    let data = if pos.len() > 1 {
        if rtr {
            return Err(CliError::usage("a remote frame has no data (use --dlc)"));
        }
        parse_text(DataType::Domain, &pos[1..].join(" "), false).map_err(|e| CliError::usage(format!("data: {e}")))?
    } else {
        Vec::new()
    };
    if data.len() > 8 {
        return Err(CliError::usage("data: at most 8 bytes"));
    }
    let mut f = CanFrame::new(id, &data);
    f.extended = ext;
    if rtr {
        f.rtr = true;
        f.dlc = a.num::<u8>("dlc")?.unwrap_or(0);
    } else if a.get("dlc").is_some() {
        return Err(CliError::usage("--dlc is for remote frames (--rtr)"));
    }
    f.validate().map_err(|e| CliError::usage(e.to_string()))?;
    let period = a.num::<u64>("period")?;
    let count = a.num::<u64>("count")?;
    let count = match (period, count) {
        (Some(_), None) => return Err(CliError::usage("--period needs --count (the number of frames)")),
        (None, Some(c)) if c > 1 => return Err(CliError::usage("--count above 1 needs --period")),
        (_, c) => c.unwrap_or(1),
    };
    if count == 0 || count > 100_000 {
        return Err(CliError::usage("--count must be 1..100000"));
    }
    let s = ctx.session()?;
    for i in 0..count {
        s.send(f).map_err(sess_err)?;
        if i + 1 < count {
            if let Some(p) = period {
                s.listen(Duration::from_millis(p), |_| true).map_err(sess_err)?;
            }
        }
    }
    Ok(json!({"id": cob(id), "ext": ext, "rtr": rtr, "dlc": f.dlc, "data": hex_bytes(f.payload()), "sent": count, "period_ms": period}))
}

fn class_name(c: MsgClass) -> &'static str {
    match c {
        MsgClass::Nmt => "nmt",
        MsgClass::Sync => "sync",
        MsgClass::Emcy => "emcy",
        MsgClass::Time => "time",
        MsgClass::Tpdo => "tpdo",
        MsgClass::Rpdo => "rpdo",
        MsgClass::SdoReq | MsgClass::SdoResp => "sdo",
        MsgClass::Heartbeat => "heartbeat",
        MsgClass::Lss => "lss",
        MsgClass::Other => "other",
    }
}

/// --duration and --count of monitor and watch; one is required, except
/// for a replay (a log file ends by itself).
fn bounds(a: &Args, replay: bool) -> Result<(Option<Duration>, Option<u64>), CliError> {
    let d = match a.get("duration") {
        Some(s) => match s.trim().parse::<f64>() {
            Ok(x) if x > 0.0 && x <= 86_400.0 => Some(Duration::from_secs_f64(x)),
            _ => return Err(CliError::usage(format!("--duration: '{s}' is not 0..86400 seconds"))),
        },
        None => None,
    };
    let c = a.num::<u64>("count")?;
    if c == Some(0) {
        return Err(CliError::usage("--count must be at least 1"));
    }
    if d.is_none() && c.is_none() && !replay {
        return Err(CliError::usage("give --duration <s> or --count <n> (the command must be bounded)"));
    }
    Ok((d, c))
}

fn line(out: &mut dyn Write, v: &Value, text: Option<String>) {
    let s = match text {
        Some(t) => format!("{t}\n"),
        None => format!("{v}\n"),
    };
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

fn monitor(ctx: &mut Ctx, pos: &[String], a: &Args, out: &mut dyn Write) -> CmdResult {
    max_args(pos, 0)?;
    let replay = ctx.is_replay();
    let (dur, count) = bounds(a, replay)?;
    let node_f = a.get("filter-node").map(|s| parse_node(s, "--filter-node")).transpose()?;
    let classes: Option<Vec<String>> = a.get("class").map(|s| s.split(',').map(|c| c.trim().to_ascii_lowercase()).collect());
    if let Some(cs) = &classes {
        let known = ["nmt", "sync", "emcy", "time", "tpdo", "rpdo", "sdo", "heartbeat", "lss", "other"];
        if let Some(bad) = cs.iter().find(|c| !known.contains(&c.as_str())) {
            return Err(CliError::usage(format!("--class: '{bad}' is not one of {}", known.join(" "))));
        }
    }
    let id_range = match a.get("id") {
        Some(s) => {
            let (lo, hi) = s.split_once('-').unwrap_or((s, s));
            Some((parse_hex_u32(lo, "--id")?, parse_hex_u32(hi, "--id")?))
        }
        None => None,
    };
    let mut nodes = HashMap::new();
    if let Some(od) = ctx.dict() {
        nodes.insert(ctx.node, od);
    }
    let dctx = DecodeContext::new(nodes);
    let mut tracker = SdoBlockTracker::new();
    let text = ctx.text;
    let s = ctx.session()?;
    let t0 = Instant::now();
    let mut n = 0u64;
    loop {
        // A replay counts --duration in log time.
        if (!replay && dur.is_some_and(|d| t0.elapsed() >= d)) || count.is_some_and(|c| n >= c) {
            break;
        }
        let Some(f) = s.receive(Duration::from_millis(2)).map_err(sess_err)? else {
            if s.finished() {
                break;
            }
            continue;
        };
        if replay && dur.is_some_and(|d| f.timestamp_us as u128 > d.as_micros()) {
            break;
        }
        let d = tracker.decode(&f, &dctx);
        if node_f.is_some_and(|x| d.node != Some(x)) {
            continue;
        }
        if classes.as_ref().is_some_and(|cs| !cs.iter().any(|c| c == class_name(d.class))) {
            continue;
        }
        if id_range.is_some_and(|(lo, hi)| f.id < lo || f.id > hi) {
            continue;
        }
        n += 1;
        let t = f.timestamp_us as f64 / 1e6;
        let id = if f.extended { format!("0x{:08X}", f.id) } else { cob(f.id) };
        let v = json!({
            "t": (t * 1e6).round() / 1e6,
            "dir": f.dir.label(),
            "id": id,
            "ext": f.extended,
            "rtr": f.rtr,
            "dlc": f.dlc,
            "data": hex_bytes(f.payload()),
            "type": d.label,
            "class": class_name(d.class),
            "node": d.node,
            "text": d.text,
        });
        let tl = text.then(|| format!("{t:>11.6} {} {id:>10} [{}] {:<23} {:<10} {}", f.dir.label(), f.dlc, hex_bytes(f.payload()), d.label, d.text));
        line(out, &v, tl);
    }
    Ok(json!({"frames": n, "duration_s": (t0.elapsed().as_secs_f64() * 1000.0).round() / 1000.0}))
}

fn pdo_read(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 0)?;
    let stop = a.num::<u16>("stop-after")?.unwrap_or(DEFAULT_PDO_SCAN_STOP).clamp(1, MAX_PDOS);
    let only = a.get("only").map(pdo_name).transpose()?;
    let cfg = read_pdos(ctx, only, stop)?;
    let od = ctx.dict();
    let mut m = node_pdo_json(&cfg, od.as_deref());
    if let (Some(lib), Some(dev)) = (&ctx.lib, &ctx.g.device) {
        if only.is_none() {
            lib.save_pdo_config(dev, &cfg).map_err(|e| CliError::new("file", e.to_string()))?;
            m.insert("saved".into(), json!(lib.pdo_config_path(dev).display().to_string()));
        }
    }
    Ok(Value::Object(m))
}

fn pdo_enable(ctx: &mut Ctx, pos: &[String], enable: bool) -> CmdResult {
    max_args(pos, 1)?;
    let target = arg(pos, 0, "<rpdoN|tpdoN|all|all-rpdo|all-tpdo>")?.to_ascii_lowercase();
    let node = ctx.node;
    let all_kind = match target.as_str() {
        "all" => Some(None),
        "all-rpdo" => Some(Some(PdoKind::Rpdo)),
        "all-tpdo" => Some(Some(PdoKind::Tpdo)),
        _ => None,
    };
    let steps = match all_kind {
        Some(kind) => {
            let cfg = read_pdos(ctx, None, DEFAULT_PDO_SCAN_STOP)?;
            plan_enable_all(node, &cfg.pdos, kind, enable)
        }
        None => {
            let (k, n) = pdo_name(&target)?;
            let p = read_one_pdo(ctx, k, n)?;
            plan_enable(node, &p, enable)
        }
    };
    let label = if enable { "enable PDO" } else { "disable PDO" };
    let rep = run_steps(ctx, label, steps, true)?;
    let mut m = Map::new();
    m.insert("node".into(), json!(node));
    m.insert("pdo".into(), json!(target));
    m.insert("enable".into(), json!(enable));
    m.insert("changed".into(), json!(!rep.done.is_empty()));
    m.insert("steps".into(), steps_json(&rep));
    finish_steps(&rep, m)
}

fn pdo_change(ctx: &mut Ctx, name: &str, change: PdoChange) -> CmdResult {
    let (k, n) = pdo_name(name)?;
    let node = ctx.node;
    let before = read_one_pdo(ctx, k, n)?;
    let after = change.apply(&before);
    let steps = plan_change(node, &before, &change);
    let rep = run_steps(ctx, "change PDO", steps, true)?;
    let od = ctx.dict();
    let mut m = Map::new();
    m.insert("node".into(), json!(node));
    m.insert("pdo".into(), json!(before.short_name()));
    m.insert("before".into(), pdo_json(&before, od.as_deref()));
    m.insert("after".into(), pdo_json(&after, od.as_deref()));
    m.insert("changed".into(), json!(!rep.done.is_empty()));
    m.insert("steps".into(), steps_json(&rep));
    finish_steps(&rep, m)
}

fn pdo_set(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 1)?;
    let name = arg(pos, 0, "<rpdoN|tpdoN>")?;
    let change = PdoChange {
        cob_id: a.get("cob-id").map(|s| parse_hex_u32(s, "--cob-id")).transpose()?,
        trans_type: a.num::<u8>("type")?,
        inhibit_100us: a.num::<u16>("inhibit")?,
        event_timer_ms: a.num::<u16>("event")?,
        sync_start: a.num::<u8>("sync-start")?,
        ..Default::default()
    };
    if change == PdoChange::default() {
        return Err(CliError::usage("pdo set: give at least one of --cob-id --type --inhibit --event --sync-start"));
    }
    if change.cob_id.is_some_and(|c| c > 0x1FFF_FFFF) {
        return Err(CliError::usage("--cob-id above 0x1FFFFFFF"));
    }
    pdo_change(ctx, name, change)
}

fn pdo_map(ctx: &mut Ctx, pos: &[String]) -> CmdResult {
    let name = arg(pos, 0, "<rpdoN|tpdoN>")?;
    if pos.len() < 2 {
        return Err(CliError::usage("missing argument: <index:sub:bits>..."));
    }
    let mapping: Vec<MapEntry> = pos[1..]
        .iter()
        .map(|s| parse_map_entry(s).ok_or_else(|| CliError::usage(format!("'{s}' is not index:sub:bits (index hex, sub and bits decimal)"))))
        .collect::<Result<_, _>>()?;
    let bits: u32 = mapping.iter().map(|m| m.bits as u32).sum();
    if bits > 64 || mapping.len() > 64 {
        return Err(CliError::usage(format!("mapping has {bits} bits; a PDO holds at most 64")));
    }
    pdo_change(ctx, name, PdoChange { mapping: Some(mapping), ..Default::default() })
}

fn heartbeat_set(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 1)?;
    let ms_s = arg(pos, 0, "<ms>")?;
    let ms = parse_int(ms_s, false).and_then(|x| u16::try_from(x).ok()).ok_or_else(|| CliError::usage(format!("heartbeat time '{ms_s}' is not 0..65535 ms")))?;
    let nodes = target_nodes(ctx, a)?;
    let rep = run_steps(ctx, "set heartbeat", heartbeat_steps(&nodes, ms), false)?;
    let mut m = Map::new();
    m.insert("ms".into(), json!(ms));
    m.insert("nodes".into(), json!(nodes));
    m.insert("steps".into(), steps_json(&rep));
    finish_steps(&rep, m)
}

fn watch(ctx: &mut Ctx, pos: &[String], a: &Args, out: &mut dyn Write) -> CmdResult {
    max_args(pos, 1)?;
    let target = arg(pos, 0, "<index:sub | pdo:index:sub>")?;
    let (from_pdo, obj) = match target.strip_prefix("pdo:") {
        Some(o) => (true, o),
        None => (false, target),
    };
    let (index, sub) = parse_obj(obj)?;
    let replay = ctx.is_replay();
    if replay && !from_pdo {
        return Err(CliError::usage("an SDO watch sends requests; with --adapter replay use watch pdo:<index:sub>"));
    }
    let (dur, count) = bounds(a, replay)?;
    let period = a.num::<u64>("period")?.unwrap_or(100);
    if period < 50 {
        return Err(CliError::usage("--period: at least 50 ms"));
    }
    if from_pdo && a.get("period").is_some() {
        return Err(CliError::usage("--period is for SDO sampling; a PDO value comes when the node sends it"));
    }
    let od = ctx.dict();
    let dt = match a.get("type") {
        Some(t) => Some(parse_type(t)?),
        None => entry_type(od.as_deref(), index, sub),
    };
    let node = ctx.node;
    let text = ctx.text;
    let mut n = 0u64;
    let done = |n: u64, t0: Instant| (!replay && dur.is_some_and(|d| t0.elapsed() >= d)) || count.is_some_and(|c| n >= c);
    let mut res = Map::new();
    res.extend(obj_head(node, index, sub, od.as_deref()));
    if !from_pdo {
        let s = ctx.session()?;
        let t0 = Instant::now();
        while !done(n, t0) {
            let start = Instant::now();
            let o = s.sdo(node, SdoRequest::Upload { index, sub }, StartOptions::default()).map_err(sess_err)?;
            let d = o.result.map_err(|e| sdo_err(&e).with_detail(json!({"samples": n})))?;
            n += 1;
            let v = value_json(dt, &d);
            let t = (s.now_us() as f64 / 1e3).round() / 1e3;
            line(out, &json!({"t": t, "value": v["value"], "raw": v["raw"]}), text.then(|| format!("{t:>10.3} {}", v["hex"].as_str().unwrap_or(""))));
            let rest = Duration::from_millis(period).saturating_sub(start.elapsed());
            if !done(n, t0) && !rest.is_zero() {
                s.listen(rest, |_| true).map_err(sess_err)?;
            }
        }
        res.insert("source".into(), json!("sdo"));
        res.insert("period_ms".into(), json!(period));
    } else {
        // A replay cannot ask the node: the PDOs come from the defaults of
        // the --eds dictionary.
        let pdos = if replay {
            let od = od.clone().ok_or_else(|| CliError::usage("watch pdo: with --adapter replay needs --eds (the PDO mapping comes from it)"))?;
            canoxide::canopen::pdo::configs_from_od(&od, node)
        } else {
            read_pdos(ctx, None, DEFAULT_PDO_SCAN_STOP)?.pdos
        };
        let mut hit = None;
        for p in pdos.iter().filter(|p| p.kind == PdoKind::Tpdo && p.valid) {
            let mut off = 0u32;
            for m in &p.mapping {
                if m.index == index && m.sub == sub {
                    hit = Some((p.clone(), off, m.bits as u32));
                    break;
                }
                off += m.bits as u32;
            }
            if hit.is_some() {
                break;
            }
        }
        let Some((p, off, bits)) = hit else {
            return Err(CliError::new("device", format!("{}:{sub} is not mapped in a valid TPDO of node {node}", hex16(index))));
        };
        let s = ctx.session()?;
        let t0 = Instant::now();
        while !done(n, t0) {
            let Some(f) = s.receive(Duration::from_millis(2)).map_err(sess_err)? else {
                if s.finished() {
                    break;
                }
                continue;
            };
            if replay && dur.is_some_and(|d| f.timestamp_us as u128 > d.as_micros()) {
                break;
            }
            if f.id != p.cob_id || f.extended != p.extended || f.rtr {
                continue;
            }
            let Some(raw) = extract_bits(f.payload(), off, bits) else { continue };
            let bytes = raw.to_le_bytes()[..(bits as usize).div_ceil(8)].to_vec();
            n += 1;
            let v = value_json(dt, &bytes);
            let t = (f.timestamp_us as f64 / 1e3).round() / 1e3;
            line(out, &json!({"t": t, "value": v["value"], "raw": v["raw"]}), text.then(|| format!("{t:>10.3} {}", v["hex"].as_str().unwrap_or(""))));
        }
        res.insert("source".into(), json!("pdo"));
        res.insert("pdo".into(), json!(p.short_name()));
        res.insert("cob_id".into(), json!(cob(p.cob_id)));
    }
    res.insert("samples".into(), json!(n));
    Ok(Value::Object(res))
}

fn identity_json(i: &LssIdentity) -> Value {
    json!({"vendor": hex32(i.vendor), "product": hex32(i.product), "revision": hex32(i.revision), "serial": hex32(i.serial)})
}

fn num_arg<T: TryFrom<i128>>(pos: &[String], i: usize, what: &str) -> Result<T, CliError> {
    let s = arg(pos, i, what)?;
    parse_int(s, false).and_then(|x| T::try_from(x).ok()).ok_or_else(|| CliError::usage(format!("{what}: bad value '{s}'")))
}

fn lss(ctx: &mut Ctx, sub: &str, pos: &[String], a: &Args) -> CmdResult {
    let cmd = match sub {
        "switch-global" => {
            max_args(pos, 1)?;
            match arg(pos, 0, "configuration|waiting")?.to_ascii_lowercase().as_str() {
                "configuration" | "config" => LssCommand::SwitchGlobal { configuration: true },
                "waiting" | "wait" => LssCommand::SwitchGlobal { configuration: false },
                w => return Err(CliError::usage(format!("'{w}' is not configuration or waiting"))),
            }
        }
        "switch-selective" => {
            max_args(pos, 4)?;
            LssCommand::SwitchSelective {
                identity: LssIdentity {
                    vendor: num_arg(pos, 0, "<vendor>")?,
                    product: num_arg(pos, 1, "<product>")?,
                    revision: num_arg(pos, 2, "<revision>")?,
                    serial: num_arg(pos, 3, "<serial>")?,
                },
            }
        }
        "inquire-identity" => LssCommand::InquireIdentity,
        "inquire-node-id" => LssCommand::InquireNodeId,
        "set-node-id" => {
            max_args(pos, 1)?;
            let id: u8 = num_arg(pos, 0, "<node-ID>")?;
            if !(1..=127).contains(&id) && id != 255 {
                return Err(CliError::usage("node-ID must be 1..127 or 255 (unconfigured)"));
            }
            LssCommand::ConfigureNodeId { node_id: id }
        }
        "set-bit-timing" => {
            max_args(pos, 1)?;
            let s = arg(pos, 0, "<kbit>")?;
            let k = s.to_ascii_lowercase().trim_end_matches('k').parse::<u32>().ok();
            let index = k.and_then(bit_timing_index).ok_or_else(|| CliError::usage(format!("'{s}' is not one of 10 20 50 100 125 250 500 800 1000 kbit/s")))?;
            LssCommand::ConfigureBitTiming { table: 0, index }
        }
        "activate-bit-timing" => {
            max_args(pos, 1)?;
            LssCommand::ActivateBitTiming { delay_ms: num_arg(pos, 0, "<delay ms>")? }
        }
        "store" => LssCommand::StoreConfiguration,
        "fastscan" => LssCommand::Fastscan,
        "identify" => LssCommand::IdentifyNonConfigured,
        other => return Err(CliError::usage(format!("unknown lss command '{other}'"))),
    };
    if matches!(cmd, LssCommand::InquireIdentity | LssCommand::InquireNodeId | LssCommand::StoreConfiguration | LssCommand::Fastscan | LssCommand::IdentifyNonConfigured) {
        max_args(pos, 0)?;
    }
    let t = a.num::<u64>("lss-timeout")?.unwrap_or(DEFAULT_LSS_TIMEOUT_MS).clamp(10, 5000);
    let s = ctx.session()?;
    s.lss.timeout_ms = t;
    let r = s.lss(cmd.clone(), &mut Ctl::default()).map_err(sess_err)?.map_err(lss_err)?;
    let mut m = Map::new();
    m.insert("service".into(), json!(cmd.label()));
    match r {
        LssResult::Done => {
            m.insert("done".into(), json!(true));
        }
        LssResult::Identity(i) => {
            m.insert("identity".into(), identity_json(&i));
        }
        LssResult::NodeId(n) => {
            m.insert("node_id".into(), json!(n));
        }
        LssResult::Found(f) => {
            m.insert("found".into(), json!(f.is_some()));
            m.insert("identity".into(), f.map(|i| identity_json(&i)).unwrap_or(Value::Null));
        }
        LssResult::NonConfigured(n) => {
            m.insert("non_configured".into(), json!(n));
        }
    }
    Ok(Value::Object(m))
}

fn dcf_save(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 1)?;
    let path = arg(pos, 0, "<out.dcf>")?;
    let (Some(od), Some(text)) = (ctx.dict(), ctx.dict_text()) else {
        return Err(CliError::usage("dcf save needs --eds (the EDS of the node)"));
    };
    let node = ctx.node;
    let file_name = Path::new(path).file_name().and_then(|f| f.to_str()).unwrap_or(path).to_string();
    let comm = DcfCommissioning {
        node_id: node,
        baudrate_kbit: ctx.g.bitrate.kbit(),
        node_name: a.get("node-name").unwrap_or("").to_string(),
        net_number: 0,
        network_name: a.get("network-name").unwrap_or("").to_string(),
    };
    let task = canoxide::dcf::DcfSaveTask::new(node, od, text, comm, file_name);
    let rep = match run_task(ctx, Box::new(task))? {
        TaskResult::DcfSave(r) => r,
        _ => return Err(internal()),
    };
    canoxide::library::write_atomic(Path::new(path), rep.text.as_bytes()).map_err(|e| CliError::new("file", e.to_string()))?;
    Ok(json!({
        "node": node,
        "file": path,
        "values": rep.values,
        "failed": rep.failed.iter().map(|(i, s, e)| json!({"index": hex16(*i), "sub": s, "error": e})).collect::<Vec<_>>(),
        "not_stored": rep.not_stored.iter().map(|(i, s)| json!({"index": hex16(*i), "sub": s})).collect::<Vec<_>>(),
        "cancelled": rep.cancelled,
    }))
}

fn dcf_restore(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 1)?;
    let path = arg(pos, 0, "<in.dcf>")?;
    let dcf = load_file_od(path)?;
    let opts = RestoreOptions {
        dry_run: a.flag("dry-run"),
        use_default_values: a.flag("default-values"),
        from: a.get("from").map(parse_index).transpose()?,
        to: a.get("to").map(parse_index).transpose()?,
        continue_on_error: a.flag("continue"),
        store: a.flag("store"),
    };
    if opts.dry_run && opts.store {
        return Err(CliError::usage("--store writes; it cannot go with --dry-run"));
    }
    let node = ctx.node;
    let rep = match run_task(ctx, Box::new(DcfRestoreTask::new(node, Arc::new(dcf), opts)))? {
        TaskResult::DcfRestore(r) => r,
        _ => return Err(internal()),
    };
    let objects: Vec<Value> = rep
        .objects
        .iter()
        .map(|o| {
            let mut v = serde_json::to_value(o).unwrap_or(Value::Null);
            v["index"] = json!(hex16(o.index));
            if let ObjectAction::Failed { abort_code: Some(c), .. } = &o.action {
                v["abort_code"] = json!(hex32(*c));
            }
            v
        })
        .collect();
    let mut m = Map::new();
    m.insert("node".into(), json!(node));
    m.insert("file".into(), json!(path));
    m.insert("dry_run".into(), json!(rep.dry_run));
    m.insert("ok".into(), json!(rep.ok));
    m.insert("written".into(), json!(rep.count(|x| matches!(x, ObjectAction::Written))));
    m.insert("would_write".into(), json!(rep.count(|x| matches!(x, ObjectAction::WouldWrite))));
    m.insert("unchanged".into(), json!(rep.count(|x| matches!(x, ObjectAction::Unchanged))));
    m.insert("skipped".into(), json!(rep.count(|x| matches!(x, ObjectAction::Skipped { .. }))));
    m.insert("failed".into(), json!(rep.count(|x| matches!(x, ObjectAction::Failed { .. }))));
    m.insert("not_run".into(), json!(rep.count(|x| matches!(x, ObjectAction::NotRun))));
    m.insert("stored".into(), json!(rep.stored));
    m.insert("cancelled".into(), json!(rep.cancelled));
    m.insert("objects".into(), Value::Array(objects));
    m.insert("pdo_steps".into(), Value::Array(rep.pdo_steps.iter().map(step_json).collect()));
    if rep.ok {
        return Ok(Value::Object(m));
    }
    let first = rep.objects.iter().find_map(|o| match &o.action {
        ObjectAction::Failed { error, abort_code } => Some((format!("{}:{}: {error}", hex16(o.index), o.sub), *abort_code)),
        _ => None,
    });
    let (msg, code) = first.or_else(|| rep.steps.first_error().map(|e| (format!("{}: {}", e.step.what, e.error.clone().unwrap_or_default()), e.abort_code))).unwrap_or(("restore failed".to_string(), None));
    m.remove("ok");
    Err(CliError { kind: if code.is_some() { "abort" } else { "device" }, message: msg, abort_code: code, detail: Some(Value::Object(m)) })
}

fn dcf_compare(ctx: &mut Ctx, pos: &[String], a: &Args) -> CmdResult {
    max_args(pos, 1)?;
    let path = arg(pos, 0, "<in.dcf>")?;
    let dcf = load_file_od(path)?;
    let node = ctx.node;
    let rep = match run_task(ctx, Box::new(DcfCompareTask::new(node, Arc::new(dcf), a.flag("default-values"))))? {
        TaskResult::DcfCompare(r) => r,
        _ => return Err(internal()),
    };
    Ok(json!({
        "node": node,
        "file": path,
        "compared": rep.compared,
        "equal": rep.differences.is_empty(),
        "cancelled": rep.cancelled,
        "differences": rep.differences.iter().map(|d| json!({
            "index": hex16(d.index), "sub": d.sub, "name": d.name, "dcf_value": d.dcf_value, "node_value": d.node_value, "error": d.error,
        })).collect::<Vec<_>>(),
    }))
}
