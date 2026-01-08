use std::env;
use std::fs;
use std::path::PathBuf;

use pvm_alto::{default_options, execute_tx_fs, FsTxConfig};
use pvm_host::HostContext;
use pvm_runtime::DeterminismOptions;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut trace_path: Option<String> = None;
    let mut trace_allow_all = false;
    let mut script_path: Option<String> = None;
    let mut input: Option<Vec<u8>> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--trace-imports" => {
                let value = args.next().ok_or_else(|| usage())?;
                trace_path = Some(value);
            }
            "--trace-allow-all" => {
                trace_allow_all = true;
            }
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(());
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--trace-imports=") {
                    trace_path = Some(value.to_owned());
                    continue;
                }
                if script_path.is_none() {
                    script_path = Some(arg);
                } else if input.is_none() {
                    input = Some(arg.into_bytes());
                } else {
                    return Err(usage().into());
                }
            }
        }
    }

    if trace_allow_all && trace_path.is_none() {
        return Err("--trace-allow-all requires --trace-imports".into());
    }

    let script_path = script_path.ok_or_else(|| usage())?;
    let input = input.unwrap_or_default();

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

    let mut options = default_options().with_source_path(script_path);
    if let Some(path) = trace_path {
        let mut det = DeterminismOptions::deterministic(None);
        det.trace_imports = true;
        det.trace_allow_all = trace_allow_all;
        det.trace_path = Some(path);
        options = options.with_determinism(det);
    }
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

fn usage() -> &'static str {
    "usage: pvm_runtime_chain_demo [--trace-imports <path>] [--trace-allow-all] <script.py> [input]"
}
