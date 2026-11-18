//! CANopenTester: a portable CANopen bench tool.
//!
//! The non-UI modules carry all logic and all tests; `ui` only draws:
//! `can` (adapters, simulator), `eds`, `canopen` (types, decode, SDO with
//! block transfer, NMT, PDO read/write, LSS, tasks), `dcf` (save, compare,
//! restore), `netinfo` (network overview), `signals` (graph data),
//! `busload`, `txlist` (transmit list), `trace` (store, views, logging),
//! `session` (blocking driver for the CLI and tests), `library`,
//! `settings`, `bus` (worker thread).

#![deny(unsafe_code)]

/// The product name in file info, CSV headers and the CLI.
pub const PRODUCT_NAME: &str = "CANopenTester";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod bus;
pub mod busload;
pub mod can;
pub mod canopen;
pub mod dcf;
pub mod eds;
pub mod library;
pub mod netinfo;
pub mod session;
pub mod settings;
pub mod signals;
pub mod trace;
pub mod txlist;
pub mod ui;
