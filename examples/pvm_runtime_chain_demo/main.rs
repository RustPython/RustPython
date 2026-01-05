use std::env;
use std::fs;
use std::path::PathBuf;

use pvm_alto::{default_options, execute_tx_fs, FsTxConfig};
use pvm_host::HostContext;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let script_path = args
        .next()
        .ok_or("usage: pvm_runtime_chain_demo <script.py> [input]")?;
    let input = args.next().map(|s| s.into_bytes()).unwrap_or_default();

    let code = fs::read(&script_path)?;

    let ctx = HostContext {
        block_height: 1,
        block_hash: [0u8; 32],
        tx_hash: [1u8; 32],
        sender: b"alice".to_vec(),
        timestamp_ms: 1_700_000_000_000,
    };

    let config = FsTxConfig {
        state_dir: PathBuf::from("tmp/pvm_state"),
        events_path: PathBuf::from("tmp/pvm_events.log"),
        gas_limit: 1_000_000,
        context: ctx,
    };

    let options = default_options().with_source_path(script_path);
    let output = execute_tx_fs(&code, &input, config, &options)?;

    println!("output_hex={}", encode_hex(&output));
    Ok(())
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
