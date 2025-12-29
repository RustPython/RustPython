use core::fmt;

pub type Bytes = Vec<u8>;

#[derive(Clone, Debug)]
pub struct HostContext {
    pub block_height: u64,
    pub block_hash: [u8; 32],
    pub tx_hash: [u8; 32],
    pub sender: Bytes,
    pub timestamp_ms: u64,
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
}
