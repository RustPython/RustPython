use std::env;
use std::fs;
use std::path::PathBuf;

use pvm_alto::{default_options, execute_tx_fs, FsTxConfig};
use pvm_host::HostContext;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut script_path: Option<String> = None;
    let mut input: Option<Vec<u8>> = None;
    let mut input_file: Option<String> = None;

    let mut sender = b"alice".to_vec();
    let mut state_dir = PathBuf::from("tmp/pvm_dex_state");
    let mut events_path = PathBuf::from("tmp/pvm_dex_events.log");
    let mut gas_limit: u64 = 1_000_000;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sender" => {
                let value = args.next().ok_or_else(|| usage())?;
                sender = value.into_bytes();
            }
            "--state-dir" => {
                let value = args.next().ok_or_else(|| usage())?;
                state_dir = PathBuf::from(value);
            }
            "--events-path" => {
                let value = args.next().ok_or_else(|| usage())?;
                events_path = PathBuf::from(value);
            }
            "--gas" => {
                let value = args.next().ok_or_else(|| usage())?;
                gas_limit = value.parse()?;
            }
            "--input-file" => {
                let value = args.next().ok_or_else(|| usage())?;
                input_file = Some(value);
            }
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(());
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--sender=") {
                    sender = value.as_bytes().to_vec();
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--state-dir=") {
                    state_dir = PathBuf::from(value);
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--events-path=") {
                    events_path = PathBuf::from(value);
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--gas=") {
                    gas_limit = value.parse()?;
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--input-file=") {
                    input_file = Some(value.to_owned());
                    continue;
                }

                if script_path.is_none() {
                    script_path = Some(arg);
                } else if input.is_none() {
                    if let Some(path) = arg.strip_prefix('@') {
                        input_file = Some(path.to_owned());
                    } else {
                        input = Some(arg.into_bytes());
                    }
                } else {
                    return Err(usage().into());
                }
            }
        }
    }

    let script_path = script_path.ok_or_else(|| usage())?;
    if input.is_some() && input_file.is_some() {
        return Err("use --input-file or input string, not both".into());
    }

    let code = fs::read(&script_path)?;
    let input = match (input, input_file) {
        (Some(bytes), None) => bytes,
        (None, Some(path)) => fs::read(path)?,
        (None, None) => Vec::new(),
        (Some(_), Some(_)) => unreachable!(),
    };

    let ctx = HostContext {
        block_height: 1,
        block_hash: [0u8; 32],
        tx_hash: [1u8; 32],
        sender,
        timestamp_ms: 1_700_000_000_000,
    };

    let config = FsTxConfig {
        state_dir,
        events_path,
        gas_limit,
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

fn usage() -> &'static str {
    "usage: pvm_dex_demo [--sender <name>] [--state-dir <path>] [--events-path <path>] [--gas <limit>] [--input-file <path>] <script.py> [input]"
}
