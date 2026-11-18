//! CANopenTester: a portable CANopen bench tool.
//!
//! The non-UI modules (`can`, `eds`, `canopen`, `library`, `settings`,
//! `trace`, `bus`) carry all logic and all tests; `ui` only draws.

#![deny(unsafe_code)]

pub mod bus;
pub mod can;
pub mod canopen;
pub mod eds;
pub mod library;
pub mod settings;
pub mod trace;
pub mod ui;
