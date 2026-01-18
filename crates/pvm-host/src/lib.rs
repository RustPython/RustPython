use core::fmt;

pub type Bytes = Vec<u8>;

#[derive(Clone, Debug)]
pub struct HostContext {
    pub block_height: u64,
    pub block_hash: [u8; 32],
    pub tx_hash: [u8; 32],
    pub sender: Bytes,
    pub timestamp_ms: u64,
    pub actor_addr: Bytes,
    pub msg_id: Bytes,
    pub nonce: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostError {
    OutOfGas,
    InvalidInput,
    NotFound,
    StorageError,
    Forbidden,
    Internal,
}

impl HostError {
    pub const fn code(&self) -> u32 {
        match self {
            HostError::OutOfGas => 1,
            HostError::InvalidInput => 2,
            HostError::NotFound => 3,
            HostError::StorageError => 4,
            HostError::Forbidden => 5,
            HostError::Internal => 6,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            HostError::OutOfGas => "out_of_gas",
            HostError::InvalidInput => "invalid_input",
            HostError::NotFound => "not_found",
            HostError::StorageError => "storage_error",
            HostError::Forbidden => "forbidden",
            HostError::Internal => "internal",
        }
    }

    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(HostError::OutOfGas),
            2 => Some(HostError::InvalidInput),
            3 => Some(HostError::NotFound),
            4 => Some(HostError::StorageError),
            5 => Some(HostError::Forbidden),
            6 => Some(HostError::Internal),
            _ => None,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "out_of_gas" => Some(HostError::OutOfGas),
            "invalid_input" => Some(HostError::InvalidInput),
            "not_found" => Some(HostError::NotFound),
            "storage_error" => Some(HostError::StorageError),
            "forbidden" => Some(HostError::Forbidden),
            "internal" => Some(HostError::Internal),
            _ => None,
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            HostError::OutOfGas => "out of gas",
            HostError::InvalidInput => "invalid input",
            HostError::NotFound => "not found",
            HostError::StorageError => "storage error",
            HostError::Forbidden => "forbidden",
            HostError::Internal => "internal error",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for HostError {}

pub type HostResult<T> = Result<T, HostError>;

pub trait HostApi {
    fn state_get(&self, key: &[u8]) -> HostResult<Option<Bytes>>;
    fn state_set(&mut self, key: &[u8], value: &[u8]) -> HostResult<()>;
    fn state_delete(&mut self, key: &[u8]) -> HostResult<()>;

    fn emit_event(&mut self, topic: &str, data: &[u8]) -> HostResult<()>;

    fn charge_gas(&mut self, amount: u64) -> HostResult<()>;
    fn gas_left(&self) -> u64;

    fn context(&self) -> HostContext;
    fn randomness(&self, domain: &[u8]) -> HostResult<[u8; 32]>;

    fn send_message(&mut self, target: &[u8], payload: &[u8]) -> HostResult<()>;
    fn schedule_timer(&mut self, height: u64, payload: &[u8]) -> HostResult<Bytes>;
    fn cancel_timer(&mut self, timer_id: &[u8]) -> HostResult<()>;
}
