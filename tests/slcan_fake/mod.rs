//! A fake SLCAN device on 127.0.0.1 for the tests: a thread with a
//! TcpListener that answers the Lawicel commands, records them, and either
//! echoes each frame or feeds it to a simulated CANopen node. No hardware,
//! no COM port, no window.

#![allow(dead_code)]

use canoxide::can::sim::SimNode;
use canoxide::can::slcan::{encode_frame, parse_line, Reply};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct Opts {
    /// Answer commands with CR / BEL and frames with z / Z (Lawicel). Off:
    /// silent like the CANable firmware (only V is answered).
    pub acks: bool,
    /// Send every transmitted frame back as a received frame.
    pub echo: bool,
    /// Add a 4-digit time stamp to the frames sent to the host.
    pub timestamps: bool,
    /// Feed the frames to a simulated node with this node-ID.
    pub sim_node: Option<u8>,
    /// Answer the frames with BEL (refused).
    pub refuse_frames: bool,
    /// Answer `L` with BEL.
    pub refuse_listen: bool,
}

pub struct Fake {
    pub addr: String,
    log: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    drop_now: Arc<AtomicBool>,
    to_host: Arc<Mutex<Vec<u8>>>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    pub fn start(opts: Opts) -> Fake {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        l.set_nonblocking(true).expect("nonblocking");
        let addr = l.local_addr().expect("addr").to_string();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let drop_now = Arc::new(AtomicBool::new(false));
        let to_host = Arc::new(Mutex::new(Vec::new()));
        let (lg, st, dn, th) = (log.clone(), stop.clone(), drop_now.clone(), to_host.clone());
        let thread = std::thread::spawn(move || {
            while !st.load(Ordering::SeqCst) {
                match l.accept() {
                    Ok((s, _)) => serve(s, &opts, &lg, &st, &dn, &th),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
                    Err(_) => break,
                }
            }
        });
        Fake { addr, log, stop, drop_now, to_host, thread: Some(thread) }
    }

    /// The commands received so far (without CR).
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// Wait until a command that starts with `prefix` is in the log.
    pub fn wait_for(&self, prefix: &str, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if self.log().iter().any(|c| c.starts_with(prefix)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// Close the open connection (the device is gone).
    pub fn drop_connection(&self) {
        self.drop_now.store(true, Ordering::SeqCst);
    }

    /// Send raw bytes to the host.
    pub fn push(&self, bytes: &[u8]) {
        self.to_host.lock().unwrap().extend_from_slice(bytes);
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(mut s: TcpStream, o: &Opts, log: &Mutex<Vec<String>>, stop: &AtomicBool, drop_now: &AtomicBool, to_host: &Mutex<Vec<u8>>) {
    s.set_nonblocking(true).expect("nonblocking");
    let _ = s.set_nodelay(true);
    let mut node = o.sim_node.map(SimNode::builtin);
    let t0 = Instant::now();
    let mut open = false;
    let mut listen = false;
    let mut line = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if drop_now.swap(false, Ordering::SeqCst) {
            let _ = s.shutdown(std::net::Shutdown::Both);
            return;
        }
        let mut out = std::mem::take(&mut *to_host.lock().unwrap());
        match s.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b != b'\r' {
                        line.push(b);
                        continue;
                    }
                    let cmd = String::from_utf8_lossy(&line).into_owned();
                    line.clear();
                    log.lock().unwrap().push(cmd.clone());
                    let ok = |out: &mut Vec<u8>, good: bool| {
                        if o.acks {
                            out.push(if good { b'\r' } else { 0x07 });
                        }
                    };
                    match cmd.as_bytes().first() {
                        Some(b'C') => {
                            ok(&mut out, open);
                            open = false;
                        }
                        Some(b'V') => out.extend_from_slice(b"V1013\r"),
                        Some(b'N') => out.extend_from_slice(b"NA123\r"),
                        Some(b'S') => ok(&mut out, !open && cmd.len() == 2 && (b'0'..=b'8').contains(&cmd.as_bytes()[1])),
                        Some(b'O') => {
                            ok(&mut out, !open);
                            open = true;
                            listen = false;
                        }
                        Some(b'L') => {
                            if o.refuse_listen {
                                out.push(0x07);
                            } else {
                                ok(&mut out, !open);
                                open = true;
                                listen = true;
                            }
                        }
                        Some(b'F') => {
                            if open {
                                out.extend_from_slice(b"F00\r");
                            } else {
                                out.push(0x07);
                            }
                        }
                        Some(b't' | b'T' | b'r' | b'R') => {
                            let ext = matches!(cmd.as_bytes()[0], b'T' | b'R');
                            let Reply::Frame { frame, .. } = parse_line(&cmd) else {
                                out.push(0x07);
                                continue;
                            };
                            if !open || listen || o.refuse_frames {
                                out.push(0x07);
                                continue;
                            }
                            if o.acks {
                                out.extend_from_slice(if ext { b"Z\r" } else { b"z\r" });
                            }
                            let mut replies = Vec::new();
                            if o.echo {
                                replies.push(frame);
                            }
                            if let Some(n) = node.as_mut() {
                                n.handle_frame(&frame, &mut replies);
                            }
                            for f in replies {
                                let mut l = encode_frame(&f).expect("valid frame");
                                if o.timestamps {
                                    l.pop();
                                    l.push_str(&format!("{:04X}\r", t0.elapsed().as_millis() % 60_000));
                                }
                                out.extend_from_slice(l.as_bytes());
                            }
                        }
                        _ => out.push(0x07),
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            Err(_) => return,
        }
        if !out.is_empty() && write_all(&mut s, &out).is_err() {
            return;
        }
    }
}

fn write_all(s: &mut TcpStream, mut d: &[u8]) -> std::io::Result<()> {
    while !d.is_empty() {
        match s.write(d) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => d = &d[n..],
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
