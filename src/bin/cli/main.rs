//! canoxide-cli: a command-line CANopen master for scripts and AI agents.
//!
//! Every result is one JSON object on stdout (`monitor`, `watch` and
//! `shell` write JSON Lines); `--text` gives tables for a human. The
//! default adapter is the built-in simulator, so nothing touches hardware
//! unless `--adapter pcan` is given, and nothing is sent unless the command
//! transmits. With `--listen-only` a transmitting command fails before the
//! adapter is opened. `help --json` prints the full command schema.

mod cmds;
mod spec;

use canoxide::can::pcan::{PcanAdapter, PCAN_USB_CHANNELS};
use canoxide::can::sim::{builtin_dictionary, SimNode, BUILTIN_EDS};
use canoxide::can::Bitrate;
use canoxide::canopen::types::parse_int;
use canoxide::eds::{self, ObjectDictionary};
use canoxide::library::Library;
use canoxide::netinfo::NetworkOverview;
use canoxide::session::Session;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

/// Name of this program in the output and the help.
pub const CLI_NAME: &str = "canoxide-cli";

/// Options without a value.
const FLAGS: &[&str] = &["listen-only", "text", "json", "all", "ext", "rtr", "dry-run", "continue", "store", "default-values"];

/// Global options that a shell line may set; the others are fixed by the
/// shell command line.
const SHELL_LINE_GLOBALS: &[&str] = &["node", "timeout", "text"];

/// Groups whose commands have two words.
const GROUPS: &[&str] = &["eds", "sdo", "od", "pdo", "heartbeat", "lss", "dcf"];

#[derive(Debug)]
pub struct CliError {
    /// usage | listen_only | file | eds | adapter | abort | timeout | protocol | device | cancelled
    pub kind: &'static str,
    pub message: String,
    pub abort_code: Option<u32>,
    /// More data of a failed command (a step report).
    pub detail: Option<Value>,
}

impl CliError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> CliError {
        CliError { kind, message: message.into(), abort_code: None, detail: None }
    }
    pub fn usage(message: impl Into<String>) -> CliError {
        CliError::new("usage", message)
    }
    pub fn with_detail(mut self, v: Value) -> CliError {
        self.detail = Some(v);
        self
    }
    pub fn exit_code(&self) -> i32 {
        match self.kind {
            "usage" | "listen_only" | "file" | "eds" => 2,
            "adapter" => 3,
            _ => 1,
        }
    }
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "ok": false,
            "error": {
                "kind": self.kind,
                "message": self.message,
                "abort_code": self.abort_code.map(|c| format!("0x{c:08X}")),
            }
        });
        if let Some(d) = &self.detail {
            v["detail"] = d.clone();
        }
        v
    }
}

pub type CmdResult = Result<Value, CliError>;

/// Parsed command line: positional words and `--name value` options.
pub struct Args {
    pub pos: Vec<String>,
    pub opts: BTreeMap<String, String>,
}

impl Args {
    pub fn parse(tokens: &[String]) -> Result<Args, CliError> {
        let mut pos = Vec::new();
        let mut opts = BTreeMap::new();
        let mut it = tokens.iter();
        while let Some(t) = it.next() {
            let Some(name) = t.strip_prefix("--").filter(|n| !n.is_empty()) else {
                pos.push(t.clone());
                continue;
            };
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (name.to_string(), None),
            };
            let value = if FLAGS.contains(&name.as_str()) {
                if inline.is_some() {
                    return Err(CliError::usage(format!("--{name} takes no value")));
                }
                String::new()
            } else {
                match inline {
                    Some(v) => v,
                    None => it.next().cloned().ok_or_else(|| CliError::usage(format!("--{name} needs a value")))?,
                }
            };
            if opts.insert(name.clone(), value).is_some() {
                return Err(CliError::usage(format!("--{name} given twice")));
            }
        }
        Ok(Args { pos, opts })
    }

    pub fn flag(&self, n: &str) -> bool {
        self.opts.contains_key(n)
    }

    pub fn get(&self, n: &str) -> Option<&str> {
        self.opts.get(n).map(|s| s.as_str())
    }

    /// An integer option (decimal or 0x hex) that fits `T`.
    pub fn num<T: TryFrom<i128>>(&self, n: &str) -> Result<Option<T>, CliError> {
        match self.get(n) {
            None => Ok(None),
            Some(s) => parse_int(s, false)
                .and_then(|x| T::try_from(x).ok())
                .map(Some)
                .ok_or_else(|| CliError::usage(format!("--{n}: bad value '{s}'"))),
        }
    }
}

/// The connection options, fixed for one process (or one shell).
#[derive(Clone, Debug)]
pub struct Global {
    pub adapter: String,
    pub channel: u32,
    pub bitrate: Bitrate,
    pub node: u8,
    pub eds: Option<PathBuf>,
    pub listen_only: bool,
    pub timeout_ms: u64,
    pub data_dir: Option<PathBuf>,
    pub device: Option<String>,
    pub text: bool,
}

pub fn parse_node(s: &str, what: &str) -> Result<u8, CliError> {
    match parse_int(s, false) {
        Some(n) if (1..=127).contains(&n) => Ok(n as u8),
        _ => Err(CliError::usage(format!("{what}: node-ID must be 1..127, not '{s}'"))),
    }
}

fn parse_bitrate(s: &str) -> Result<Bitrate, CliError> {
    let l = s.trim().to_ascii_lowercase();
    let k = if let Some(m) = l.strip_suffix('m') {
        m.parse::<u32>().ok().map(|x| x * 1000)
    } else {
        l.strip_suffix('k').unwrap_or(&l).parse::<u32>().ok()
    };
    k.and_then(Bitrate::from_kbit)
        .ok_or_else(|| CliError::usage(format!("--bitrate: '{s}' is not one of 10k 20k 50k 125k 250k 500k 800k 1M")))
}

fn parse_channel(s: &str) -> Result<u32, CliError> {
    let l = s.trim().to_ascii_lowercase();
    if let Some(n) = l.strip_prefix("usb").and_then(|n| n.parse::<usize>().ok()) {
        if (1..=8).contains(&n) {
            return Ok(PCAN_USB_CHANNELS[n - 1] as u32);
        }
    }
    match parse_int(&l, false) {
        Some(h) if PCAN_USB_CHANNELS.iter().any(|&c| c as i128 == h) => Ok(h as u32),
        _ => Err(CliError::usage(format!("--channel: '{s}' is not usb1..usb8"))),
    }
}

impl Global {
    fn from_args(a: &Args) -> Result<Global, CliError> {
        let adapter = a.get("adapter").unwrap_or("sim").to_ascii_lowercase();
        if adapter != "sim" && adapter != "pcan" {
            return Err(CliError::usage(format!("--adapter: '{adapter}' is not sim or pcan")));
        }
        let g = Global {
            adapter,
            channel: parse_channel(a.get("channel").unwrap_or("usb1"))?,
            bitrate: parse_bitrate(a.get("bitrate").unwrap_or("250k"))?,
            node: match a.get("node") {
                Some(n) => parse_node(n, "--node")?,
                None => 1,
            },
            eds: a.get("eds").map(PathBuf::from),
            listen_only: a.flag("listen-only"),
            timeout_ms: a.num::<u64>("timeout")?.unwrap_or(1000).clamp(1, 600_000),
            data_dir: a.get("data-dir").map(PathBuf::from),
            device: a.get("device").map(str::to_string),
            text: a.flag("text"),
        };
        if g.device.is_some() && g.data_dir.is_none() {
            return Err(CliError::usage("--device needs --data-dir (the data folder of the device library)"));
        }
        Ok(g)
    }
}

/// The state of one process or one shell.
pub struct Ctx {
    pub g: Global,
    /// Target node, timeout and output form of the current command.
    pub node: u8,
    pub timeout_ms: u64,
    pub text: bool,
    /// The dictionary of --eds (or of the device).
    pub od: Option<Arc<ObjectDictionary>>,
    pub eds_text: Option<String>,
    pub lib: Option<Library>,
    pub session: Option<Session>,
    pub overview: NetworkOverview,
    builtin: Option<Arc<ObjectDictionary>>,
}

impl Ctx {
    fn new(mut g: Global, a: &Args) -> Result<Ctx, CliError> {
        let mut lib = None;
        if let Some(dir) = &g.data_dir {
            let l = Library::open(dir).map_err(|e| CliError::new("file", format!("--data-dir: {e}")))?;
            if let Some(name) = &g.device {
                let i = l.find_device(name).ok_or_else(|| CliError::usage(format!("--device: no device named '{name}' in {}", dir.display())))?;
                let d = l.devices[i].clone();
                if a.get("node").is_none() {
                    g.node = d.node_id;
                }
                if a.get("bitrate").is_none() {
                    g.bitrate = Bitrate::from_kbit(d.bitrate_kbit).unwrap_or(g.bitrate);
                }
                if g.eds.is_none() {
                    g.eds = d.eds_file.as_ref().map(|f| l.eds_path(f));
                }
            }
            lib = Some(l);
        }
        let (od, eds_text) = match &g.eds {
            Some(p) => {
                let bytes = std::fs::read(p).map_err(|e| CliError::new("eds", format!("--eds {}: {e}", p.display())))?;
                let mut od = eds::parse_bytes(&bytes);
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                od.is_dcf |= ext == "dcf" || ext == "xdc";
                if od.objects.is_empty() {
                    return Err(CliError::new("eds", format!("--eds {}: no objects found", p.display())));
                }
                (Some(Arc::new(od)), Some(eds::decode_text(&bytes)))
            }
            None => (None, None),
        };
        Ok(Ctx { node: g.node, timeout_ms: g.timeout_ms, text: g.text, g, od, eds_text, lib, session: None, overview: NetworkOverview::new(), builtin: None })
    }

    pub fn is_sim(&self) -> bool {
        self.g.adapter == "sim"
    }

    /// The dictionary for names and types: --eds, or the dictionary of the
    /// built-in simulated node.
    pub fn dict(&mut self) -> Option<Arc<ObjectDictionary>> {
        if let Some(od) = &self.od {
            return Some(od.clone());
        }
        if !self.is_sim() {
            return None;
        }
        Some(self.builtin.get_or_insert_with(|| Arc::new(builtin_dictionary())).clone())
    }

    /// The EDS text of `dict()`.
    pub fn dict_text(&self) -> Option<String> {
        match &self.eds_text {
            Some(t) => Some(t.clone()),
            None if self.is_sim() => Some(BUILTIN_EDS.to_string()),
            None => None,
        }
    }

    /// The open session (opened on first use). Sends nothing.
    pub fn session(&mut self) -> Result<&mut Session, CliError> {
        if self.session.is_none() {
            let s = if self.is_sim() {
                let node = match &self.od {
                    Some(od) => SimNode::new(self.g.node, od.clone()),
                    None => SimNode::builtin(self.g.node),
                };
                Session::sim(node, self.g.listen_only)
            } else {
                let a = PcanAdapter::load().map_err(|e| CliError::new("adapter", format!("PCAN driver: {e}")))?;
                Session::open(Box::new(a), self.g.channel, self.g.bitrate, self.g.listen_only).map_err(|e| CliError::new("adapter", format!("open channel: {e}")))?
            };
            self.session = Some(s);
        }
        let t = self.timeout_ms;
        let s = self.session.as_mut().expect("opened above");
        s.sdo.timeout_ms = t;
        Ok(s)
    }

    /// Feed the frames of the last command into the network overview.
    fn after_command(&mut self) {
        if let Some(s) = self.session.as_mut() {
            for f in s.take_frames() {
                self.overview.on_frame(&f);
            }
        }
    }
}

/// The command of the positional words: (spec, remaining words).
fn resolve(pos: &[String]) -> Result<Option<(&'static spec::Spec, Vec<String>)>, CliError> {
    let Some(first) = pos.first() else { return Ok(None) };
    let first = first.to_ascii_lowercase();
    if let Some(second) = pos.get(1) {
        if let Some(s) = spec::find(&format!("{first} {}", second.to_ascii_lowercase())) {
            return Ok(Some((s, pos[2..].to_vec())));
        }
    }
    if let Some(s) = spec::find(&first) {
        return Ok(Some((s, pos[1..].to_vec())));
    }
    if GROUPS.contains(&first.as_str()) {
        let subs: Vec<&str> = spec::COMMANDS.iter().filter_map(|c| c.name.strip_prefix(&format!("{first} "))).collect();
        return Err(CliError::usage(format!("'{first}' needs a sub-command: {}", subs.join(", "))));
    }
    Err(CliError::usage(format!("unknown command '{first}' (see `{CLI_NAME} help`)")))
}

/// Check that every option is known for this command.
fn check_options(a: &Args, s: &spec::Spec, globals: &[&str]) -> Result<(), CliError> {
    for name in a.opts.keys() {
        let n = name.as_str();
        if s.options.iter().any(|o| o.0 == n) {
            continue;
        }
        if spec::GLOBAL.iter().any(|o| o.0 == n) {
            if globals.contains(&n) {
                continue;
            }
            return Err(CliError::usage(format!("--{n} is fixed by the shell command line; give it there")));
        }
        return Err(CliError::usage(format!("unknown option --{n} for '{}' (see `{CLI_NAME} help {}`)", s.name, s.name)));
    }
    Ok(())
}

fn all_globals() -> Vec<&'static str> {
    spec::GLOBAL.iter().map(|o| o.0).collect()
}

/// Write one result (JSON line or text).
fn emit(out: &mut dyn Write, text: bool, name: &str, r: &CmdResult) {
    let v = match r {
        Ok(v) => {
            let mut m = Map::new();
            m.insert("ok".into(), Value::Bool(true));
            m.insert("command".into(), Value::String(name.to_string()));
            if let Value::Object(o) = v {
                for (k, x) in o {
                    m.insert(k.clone(), x.clone());
                }
            }
            Value::Object(m)
        }
        Err(e) => e.to_json(),
    };
    let s = if text { render_text(&v) } else { format!("{v}\n") };
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::Null => "-".to_string(),
        Value::String(s) => s.clone(),
        Value::Array(a) if a.iter().all(|x| !x.is_object() && !x.is_array()) => a.iter().map(scalar_text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

fn table(rows: &[Value], indent: &str, out: &mut String) {
    let mut cols: Vec<String> = Vec::new();
    for r in rows {
        if let Value::Object(o) = r {
            for k in o.keys() {
                if !cols.contains(k) {
                    cols.push(k.clone());
                }
            }
        }
    }
    let cells: Vec<Vec<String>> = rows.iter().map(|r| cols.iter().map(|c| r.get(c).map(scalar_text).unwrap_or_default()).collect()).collect();
    let widths: Vec<usize> = cols.iter().enumerate().map(|(i, c)| cells.iter().map(|r| r[i].chars().count()).chain([c.len()]).max().unwrap_or(0).min(60)).collect();
    let line = |vals: &[String]| -> String {
        let parts: Vec<String> = vals.iter().zip(&widths).map(|(v, w)| format!("{v:<w$}")).collect();
        format!("{indent}{}\n", parts.join("  ").trim_end())
    };
    out.push_str(&line(&cols));
    for r in &cells {
        out.push_str(&line(r));
    }
}

fn text_into(v: &Value, indent: &str, out: &mut String) {
    let Value::Object(o) = v else {
        out.push_str(&format!("{indent}{}\n", scalar_text(v)));
        return;
    };
    for (k, x) in o {
        match x {
            Value::Object(_) => {
                out.push_str(&format!("{indent}{k}:\n"));
                text_into(x, &format!("{indent}  "), out);
            }
            Value::Array(a) if a.iter().any(|e| e.is_object()) => {
                out.push_str(&format!("{indent}{k}: {} item(s)\n", a.len()));
                table(a, &format!("{indent}  "), out);
            }
            _ => out.push_str(&format!("{indent}{k}: {}\n", scalar_text(x))),
        }
    }
}

/// A JSON result as key/value lines and tables.
pub fn render_text(v: &Value) -> String {
    let mut s = String::new();
    text_into(v, "", &mut s);
    s
}

/// Split a shell line into words ("..." and '...' keep spaces).
fn split_line(line: &str) -> Result<Vec<String>, CliError> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut have = false;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                have = true;
            }
            None if c.is_whitespace() => {
                if have || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            None => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err(CliError::usage("unclosed quote"));
    }
    if have || !cur.is_empty() {
        words.push(cur);
    }
    Ok(words)
}

/// Run one command (after the listen-only check).
fn run_one(ctx: &mut Ctx, s: &'static spec::Spec, rest: &[String], a: &Args, out: &mut dyn Write) -> CmdResult {
    if s.sends && ctx.g.listen_only {
        return Err(CliError::new("listen_only", format!("'{}' transmits on the bus; refused with --listen-only (nothing was opened or sent)", s.name)));
    }
    let r = cmds::dispatch(ctx, s.name, rest, a, out);
    ctx.after_command();
    r
}

fn shell(ctx: &mut Ctx, a: &Args, out: &mut dyn Write) -> i32 {
    let lines: Vec<String> = match a.get("script") {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(t) => t.lines().map(str::to_string).collect(),
            Err(e) => {
                let r = Err(CliError::new("file", format!("--script {p}: {e}")));
                emit(out, ctx.g.text, "shell", &r);
                return 2;
            }
        },
        None => Vec::new(),
    };
    let from_stdin = a.get("script").is_none();
    let stdin = std::io::stdin();
    let mut stdin_lines = stdin.lock().lines();
    let mut script = lines.into_iter();
    let mut code = 0;
    loop {
        let line = if from_stdin {
            match stdin_lines.next() {
                Some(Ok(l)) => l,
                _ => break,
            }
        } else {
            match script.next() {
                Some(l) => l,
                None => break,
            }
        };
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t == "exit" || t == "quit" {
            break;
        }
        ctx.node = ctx.g.node;
        ctx.timeout_ms = ctx.g.timeout_ms;
        ctx.text = ctx.g.text;
        let mut name = "shell";
        let r = (|| -> CmdResult {
            let words = split_line(t)?;
            let la = Args::parse(&words)?;
            let Some((s, rest)) = resolve(&la.pos)? else { return Err(CliError::usage("empty command")) };
            name = s.name;
            if s.name == "shell" {
                return Err(CliError::usage("shell cannot run inside a shell"));
            }
            check_options(&la, s, SHELL_LINE_GLOBALS)?;
            if let Some(n) = la.get("node") {
                ctx.node = parse_node(n, "--node")?;
            }
            if let Some(t) = la.num::<u64>("timeout")? {
                ctx.timeout_ms = t.clamp(1, 600_000);
            }
            ctx.text |= la.flag("text");
            run_one(ctx, s, &rest, &la, out)
        })();
        if let Err(e) = &r {
            code = e.exit_code();
        }
        emit(out, ctx.text, name, &r);
    }
    code
}

fn run(tokens: &[String], out: &mut dyn Write) -> i32 {
    let text = tokens.iter().any(|t| t == "--text");
    let fail = |out: &mut dyn Write, name: &str, e: CliError| -> i32 {
        let code = e.exit_code();
        emit(out, text, name, &Err(e));
        code
    };
    let a = match Args::parse(tokens) {
        Ok(a) => a,
        Err(e) => return fail(out, "", e),
    };
    let (s, rest) = match resolve(&a.pos) {
        Ok(Some(x)) => x,
        Ok(None) => {
            let _ = out.write_all(spec::usage_text(CLI_NAME, canoxide::VERSION).as_bytes());
            return 2;
        }
        Err(e) => return fail(out, "", e),
    };
    if let Err(e) = check_options(&a, s, &all_globals()) {
        return fail(out, s.name, e);
    }
    if s.name == "help" {
        return cmds::help(&rest, &a, out);
    }
    if s.sends && a.flag("listen-only") {
        return fail(out, s.name, CliError::new("listen_only", format!("'{}' transmits on the bus; refused with --listen-only (nothing was opened or sent)", s.name)));
    }
    let g = match Global::from_args(&a) {
        Ok(g) => g,
        Err(e) => return fail(out, s.name, e),
    };
    let mut ctx = match Ctx::new(g, &a) {
        Ok(c) => c,
        Err(e) => return fail(out, s.name, e),
    };
    if s.name == "shell" {
        if !rest.is_empty() {
            return fail(out, s.name, CliError::usage("shell takes no arguments"));
        }
        let code = shell(&mut ctx, &a, out);
        if let Some(sess) = ctx.session.as_mut() {
            sess.close();
        }
        return code;
    }
    let r = run_one(&mut ctx, s, &rest, &a, out);
    if let Some(sess) = ctx.session.as_mut() {
        sess.close();
    }
    let code = r.as_ref().err().map(|e| e.exit_code()).unwrap_or(0);
    emit(out, ctx.text, s.name, &r);
    code
}

fn main() {
    let tokens: Vec<String> = std::env::args().skip(1).collect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let code = run(&tokens, &mut out);
    let _ = out.flush();
    std::process::exit(code);
}
