//! The SLCAN adapter over TCP against a fake SLCAN device on 127.0.0.1
//! (tests/slcan_fake): the open sequence, a frame exchange, listen only, a
//! refused frame and a lost connection. The COM transport is not tested
//! here (it needs a real port).

mod slcan_fake;

use canoxide::bus::*;
use canoxide::can::slcan::{SlcanAdapter, SlcanTarget};
use canoxide::can::{Bitrate, CanAdapter, CanFrame};
use slcan_fake::{Fake, Opts};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn adapter(f: &Fake) -> SlcanAdapter {
    SlcanAdapter::new(SlcanTarget::Tcp(f.addr.clone()))
}

fn recv(a: &mut SlcanAdapter, timeout: Duration) -> Option<CanFrame> {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if let Some(f) = a.receive(Duration::from_millis(20)).expect("receive") {
            return Some(f);
        }
    }
    None
}

fn lawicel() -> Opts {
    Opts { acks: true, ..Opts::default() }
}

#[test]
fn open_sequence_of_a_lawicel_device() {
    let f = Fake::start(lawicel());
    let mut a = adapter(&f);
    a.open(0, Bitrate::K500, false).unwrap();
    assert_eq!(f.log(), ["C", "V", "S6", "O"], "opening sends no frame");
    assert!(a.answers_commands());
    assert_eq!(a.version(), Some("1013"));
    assert_eq!(a.name(), format!("SLCAN {}", f.addr));
    a.close();
    let end = Instant::now() + Duration::from_secs(2);
    while f.log().len() < 5 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(f.log(), ["C", "V", "S6", "O", "C"], "close sends C");
}

#[test]
fn open_sequence_of_a_silent_canable_device() {
    let f = Fake::start(Opts::default());
    let mut a = adapter(&f);
    a.open(0, Bitrate::M1, false).unwrap();
    assert_eq!(f.log(), ["C", "V", "S8", "O"]);
    assert!(!a.answers_commands());
    // status() asks for F only on a device that answers commands.
    assert_eq!(a.status(), "OK");
    std::thread::sleep(Duration::from_millis(50));
    assert!(!f.log().iter().any(|c| c == "F"));
}

#[test]
fn every_bitrate_code() {
    let f = Fake::start(lawicel());
    for (b, s) in [(Bitrate::K10, "S0"), (Bitrate::K20, "S1"), (Bitrate::K50, "S2"), (Bitrate::K125, "S4"), (Bitrate::K250, "S5"), (Bitrate::K800, "S7")] {
        let mut a = adapter(&f);
        a.open(0, b, false).unwrap();
        a.close();
        drop(a);
        assert!(f.log().iter().any(|c| c == s), "{b:?}: {:?}", f.log());
    }
}

#[test]
fn frame_exchange_with_time_stamps() {
    let f = Fake::start(Opts { acks: true, echo: true, timestamps: true, ..Opts::default() });
    let mut a = adapter(&f);
    a.open(0, Bitrate::K250, false).unwrap();
    let std_f = CanFrame::new(0x123, &[0xAA, 0xBB]);
    let mut ext_f = CanFrame::new(0x1ABC_DEF0, &[1, 2, 3, 4, 5, 6, 7, 8]);
    ext_f.extended = true;
    let mut rtr_f = CanFrame::new(0x7FF, &[]);
    rtr_f.rtr = true;
    rtr_f.dlc = 3;
    let empty = CanFrame::new(0x000, &[]);
    for sent in [&std_f, &ext_f, &rtr_f, &empty] {
        a.send(sent).unwrap();
        let got = recv(&mut a, Duration::from_secs(2)).expect("echo");
        assert_eq!((got.id, got.extended, got.rtr, got.dlc, got.payload()), (sent.id, sent.extended, sent.rtr, sent.dlc, sent.payload()));
    }
    let log = f.log();
    for l in ["t1232AABB", "T1ABCDEF080102030405060708", "r7FF3", "t0000"] {
        assert!(log.iter().any(|c| c == l), "{l} not sent: {log:?}");
    }
    assert_eq!(a.unknown_lines(), 0);
    // F is polled on a device that answers; its answer is read with the frames.
    assert_eq!(a.status(), "OK");
    assert!(f.wait_for("F", Duration::from_secs(2)));
    assert!(a.receive(Duration::from_millis(50)).unwrap().is_none());
    assert_eq!(a.status(), "OK");
}

#[test]
fn listen_only_sends_l_and_refuses_to_transmit() {
    let f = Fake::start(lawicel());
    let mut a = adapter(&f);
    a.open(0, Bitrate::K125, true).unwrap();
    assert_eq!(f.log(), ["C", "V", "S4", "L"]);
    let e = a.send(&CanFrame::new(0x123, &[1])).unwrap_err();
    assert!(e.0.contains("listen-only"), "{e}");
    std::thread::sleep(Duration::from_millis(50));
    assert!(!f.log().iter().any(|c| c.starts_with('t')));
}

#[test]
fn a_device_without_listen_only_is_an_error() {
    let f = Fake::start(Opts { acks: true, refuse_listen: true, ..Opts::default() });
    let mut a = adapter(&f);
    let e = a.open(0, Bitrate::K125, true).unwrap_err();
    assert!(e.0.contains("listen-only"), "{e}");
    assert!(!f.log().iter().any(|c| c == "O"), "no fallback to O: {:?}", f.log());
}

#[test]
fn a_refused_frame_is_reported_once_and_send_does_not_wait() {
    let f = Fake::start(Opts { acks: true, refuse_frames: true, ..Opts::default() });
    let mut a = adapter(&f);
    a.open(0, Bitrate::K250, false).unwrap();
    let t = Instant::now();
    a.send(&CanFrame::new(0x100, &[1])).unwrap();
    assert!(t.elapsed() < Duration::from_millis(100), "send waited {:?}", t.elapsed());
    let end = Instant::now() + Duration::from_secs(2);
    let e = loop {
        match a.receive(Duration::from_millis(20)) {
            Err(e) => break e,
            Ok(_) if Instant::now() < end => {}
            Ok(_) => panic!("no error for the refused frame"),
        }
    };
    assert!(e.0.contains("refused 1 frame"), "{e}");
    assert!(a.receive(Duration::from_millis(50)).unwrap().is_none(), "reported once");
    assert!(a.status().contains("1 frame(s) refused"));
    assert_eq!(a.refused_frames(), 1);
}

#[test]
fn unknown_lines_and_partial_lines_are_handled() {
    let f = Fake::start(Opts::default());
    let mut a = adapter(&f);
    a.open(0, Bitrate::K250, false).unwrap();
    f.push(b"Q12\rt10");
    std::thread::sleep(Duration::from_millis(30));
    assert!(a.receive(Duration::from_millis(30)).unwrap().is_none());
    f.push(b"01AB\r");
    let got = recv(&mut a, Duration::from_secs(2)).expect("frame");
    assert_eq!((got.id, got.payload()), (0x100, &[0xAB][..]));
    assert_eq!(a.unknown_lines(), 1);
}

#[test]
fn a_lost_connection_gives_the_same_error_every_time() {
    let f = Fake::start(Opts::default());
    let mut a = adapter(&f);
    a.open(0, Bitrate::K250, false).unwrap();
    f.drop_connection();
    let end = Instant::now() + Duration::from_secs(3);
    let e = loop {
        match a.receive(Duration::from_millis(20)) {
            Err(e) => break e,
            Ok(_) if Instant::now() < end => {}
            Ok(_) => panic!("the lost connection is not seen"),
        }
    };
    assert!(e.0.contains("connection lost"), "{e}");
    for _ in 0..3 {
        assert_eq!(a.receive(Duration::from_millis(5)).unwrap_err().0, e.0);
    }
    assert_eq!(a.send(&CanFrame::new(1, &[])).unwrap_err().0, e.0);
    assert_eq!(a.status(), "Connection lost");
}

#[test]
fn a_lost_connection_is_one_bus_error() {
    let f = Fake::start(Opts::default());
    let bus = BusHandle::spawn(Arc::new(|| {}));
    bus.send(BusCommand::Connect { adapter: AdapterChoice::Slcan(SlcanTarget::Tcp(f.addr.clone())), channel: 0, bitrate: Bitrate::K250, listen_only: false });
    let end = Instant::now() + Duration::from_secs(5);
    let mut connected = false;
    while Instant::now() < end && !connected {
        while let Some(e) = bus.try_recv() {
            if let BusEvent::Error(t) = &e {
                panic!("connect failed: {t}");
            }
            connected |= matches!(e, BusEvent::Connected { .. });
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(connected);
    f.drop_connection();
    let end = Instant::now() + Duration::from_secs(2);
    let mut errors = Vec::new();
    while Instant::now() < end {
        while let Some(e) = bus.try_recv() {
            if let BusEvent::Error(t) = e {
                errors.push(t);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("connection lost"), "{errors:?}");
    let status = bus.shared.lock().unwrap().status.clone();
    assert_eq!(status, "Connection lost");
    bus.send(BusCommand::Shutdown);
}

#[test]
fn a_device_that_does_not_answer_v_is_refused() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let t = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        drop(s);
    });
    let mut a = SlcanAdapter::new(SlcanTarget::Tcp(addr));
    let e = a.open(0, Bitrate::K250, false).unwrap_err();
    assert!(e.0.contains("no answer to the version command"), "{e}");
    t.join().unwrap();
}

#[test]
fn nothing_listening_is_an_open_error() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    drop(l);
    let mut a = SlcanAdapter::new(SlcanTarget::Tcp(addr.clone()));
    let t = Instant::now();
    let e = a.open(0, Bitrate::K250, false).unwrap_err();
    assert!(e.0.starts_with(&format!("SLCAN {addr}")), "{e}");
    assert!(t.elapsed() < Duration::from_secs(4));
}
