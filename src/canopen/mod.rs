//! CANopen protocol parts (CiA 301): data types, frame decoder, SDO client,
//! NMT and PDO mapping. No I/O in here.

pub mod decode;
pub mod nmt;
pub mod pdo;
pub mod sdo;
pub mod types;

pub const COB_NMT: u32 = 0x000;
pub const COB_SYNC: u32 = 0x080;
pub const COB_EMCY: u32 = 0x080;
pub const COB_TIME: u32 = 0x100;
pub const COB_SDO_TX: u32 = 0x580;
pub const COB_SDO_RX: u32 = 0x600;
pub const COB_HEARTBEAT: u32 = 0x700;
pub const COB_LSS_SLAVE: u32 = 0x7E4;
pub const COB_LSS_MASTER: u32 = 0x7E5;

/// SDO abort codes of CiA 301 as text.
pub fn abort_text(code: u32) -> &'static str {
    match code {
        0x0503_0000 => "Toggle bit not alternated",
        0x0504_0000 => "SDO protocol timed out",
        0x0504_0001 => "Client/server command specifier not valid or unknown",
        0x0504_0002 => "Invalid block size",
        0x0504_0003 => "Invalid sequence number",
        0x0504_0004 => "CRC error",
        0x0504_0005 => "Out of memory",
        0x0601_0000 => "Unsupported access to an object",
        0x0601_0001 => "Attempt to read a write only object",
        0x0601_0002 => "Attempt to write a read only object",
        0x0602_0000 => "Object does not exist in the object dictionary",
        0x0604_0041 => "Object cannot be mapped to the PDO",
        0x0604_0042 => "The number and length of the objects to be mapped would exceed PDO length",
        0x0604_0043 => "General parameter incompatibility reason",
        0x0604_0047 => "General internal incompatibility in the device",
        0x0606_0000 => "Access failed due to a hardware error",
        0x0607_0010 => "Data type does not match, length of service parameter does not match",
        0x0607_0012 => "Data type does not match, length of service parameter too high",
        0x0607_0013 => "Data type does not match, length of service parameter too low",
        0x0609_0011 => "Sub-index does not exist",
        0x0609_0030 => "Invalid value for parameter (download only)",
        0x0609_0031 => "Value of parameter written too high (download only)",
        0x0609_0032 => "Value of parameter written too low (download only)",
        0x0609_0036 => "Maximum value is less than minimum value",
        0x060A_0023 => "Resource not available: SDO connection",
        0x0800_0000 => "General error",
        0x0800_0020 => "Data cannot be transferred or stored to the application",
        0x0800_0021 => "Data cannot be transferred or stored to the application because of local control",
        0x0800_0022 => "Data cannot be transferred or stored to the application because of the present device state",
        0x0800_0023 => "Object dictionary dynamic generation fails or no object dictionary is present",
        0x0800_0024 => "No data available",
        _ => "Unknown abort code",
    }
}
