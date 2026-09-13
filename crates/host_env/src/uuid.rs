//! Windows `UuidCreateSequential` — the `_uuid` door `uuid.getnode()` reads.

use windows_sys::Win32::System::Rpc::{
    RPC_S_OK, RPC_S_UUID_LOCAL_ONLY, RPC_S_UUID_NO_ADDRESS, UuidCreateSequential,
};
use windows_sys::core::GUID;

pub const STATUS_OK: i32 = RPC_S_OK;
pub const STATUS_LOCAL_ONLY: i32 = RPC_S_UUID_LOCAL_ONLY;
pub const STATUS_NO_ADDRESS: i32 = RPC_S_UUID_NO_ADDRESS;

#[derive(Clone, Copy, Debug)]
pub struct SequentialUuid {
    pub bytes: [u8; 16],
    pub status: i32,
}

/// One `UuidCreateSequential` call. `status` is the raw RPC status.
#[must_use]
pub fn create_sequential() -> SequentialUuid {
    let mut uuid = GUID::from_u128(0);
    // SAFETY: `uuid` is a live, aligned `GUID` the callee only writes into.
    let status = unsafe { UuidCreateSequential(&raw mut uuid) };
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&uuid.data1.to_le_bytes());
    bytes[4..6].copy_from_slice(&uuid.data2.to_le_bytes());
    bytes[6..8].copy_from_slice(&uuid.data3.to_le_bytes());
    bytes[8..16].copy_from_slice(&uuid.data4);
    SequentialUuid { bytes, status }
}

#[must_use]
pub fn has_stable_node() -> bool {
    create_sequential().status == STATUS_OK
}
