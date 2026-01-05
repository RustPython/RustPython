use std::fs;
use std::io::Write;
use std::path::PathBuf;

use pvm_host::{Bytes, HostApi, HostContext, HostError, HostResult};
use pvm_runtime::{execute_tx_with_options, ExecutionOptions};

pub struct FsHost {
    state_dir: PathBuf,
    events_path: PathBuf,
    gas_left: u64,
    context: HostContext,
    randomness_seed: [u8; 32],
}

impl FsHost {
    pub fn new(
        state_dir: impl Into<PathBuf>,
        events_path: impl Into<PathBuf>,
        gas_limit: u64,
        context: HostContext,
    ) -> Result<Self, HostError> {
        let state_dir = state_dir.into();
        let events_path = events_path.into();

        fs::create_dir_all(&state_dir).map_err(|_| HostError::StorageError)?;
        if let Some(parent) = events_path.parent() {
            fs::create_dir_all(parent).map_err(|_| HostError::StorageError)?;
        }

        Ok(Self {
            state_dir,
            events_path,
            gas_left: gas_limit,
            randomness_seed: context.tx_hash,
            context,
        })
    }

    pub fn with_randomness_seed(mut self, seed: [u8; 32]) -> Self {
        self.randomness_seed = seed;
        self
    }

    fn key_path(&self, key: &[u8]) -> PathBuf {
        let name = if key.is_empty() {
            "__empty__".to_owned()
        } else {
            encode_hex(key)
        };
        self.state_dir.join(name)
    }
}

impl HostApi for FsHost {
    fn state_get(&self, key: &[u8]) -> HostResult<Option<Bytes>> {
        let path = self.key_path(key);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(HostError::StorageError),
        }
    }

    fn state_set(&mut self, key: &[u8], value: &[u8]) -> HostResult<()> {
        let path = self.key_path(key);
        fs::write(path, value).map_err(|_| HostError::StorageError)
    }

    fn state_delete(&mut self, key: &[u8]) -> HostResult<()> {
        let path = self.key_path(key);
        match fs::remove_file(path) {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(HostError::StorageError),
        }
    }

    fn emit_event(&mut self, topic: &str, data: &[u8]) -> HostResult<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.events_path)
            .map_err(|_| HostError::StorageError)?;
        let line = format!("{}:{}\n", topic, encode_hex(data));
        file.write_all(line.as_bytes())
            .map_err(|_| HostError::StorageError)
    }

    fn charge_gas(&mut self, amount: u64) -> HostResult<()> {
        if amount > self.gas_left {
            return Err(HostError::OutOfGas);
        }
        self.gas_left -= amount;
        Ok(())
    }

    fn gas_left(&self) -> u64 {
        self.gas_left
    }

    fn context(&self) -> HostContext {
        self.context.clone()
    }

    fn randomness(&self, domain: &[u8]) -> HostResult<[u8; 32]> {
        Ok(pseudo_random(&self.randomness_seed, domain))
    }
}

pub struct FsTxConfig {
    pub state_dir: PathBuf,
    pub events_path: PathBuf,
    pub gas_limit: u64,
    pub context: HostContext,
}

pub fn execute_tx_fs(
    code: &[u8],
    input: &[u8],
    config: FsTxConfig,
    options: &ExecutionOptions,
) -> Result<Bytes, HostError> {
    let mut host = FsHost::new(
        config.state_dir,
        config.events_path,
        config.gas_limit,
        config.context,
    )?;
    execute_tx_with_options(&mut host, code, input, options)
}

pub fn default_options() -> ExecutionOptions {
    ExecutionOptions::default()
        .with_source_path("contract.py")
        .with_entrypoint("main")
        .deterministic()
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn fnv1a64(mut hash: u64, bytes: &[u8]) -> u64 {
    const FNV_PRIME: u64 = 0x00000100000001b3;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn pseudo_random(seed: &[u8; 32], domain: &[u8]) -> [u8; 32] {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    let mut out = [0u8; 32];
    for (idx, chunk) in out.chunks_exact_mut(8).enumerate() {
        let mut hash = FNV_OFFSET;
        hash = fnv1a64(hash, seed);
        hash = fnv1a64(hash, domain);
        hash = fnv1a64(hash, &(idx as u64).to_le_bytes());
        chunk.copy_from_slice(&hash.to_le_bytes());
    }
    out
}
