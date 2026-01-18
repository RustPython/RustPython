use std::env;
use std::fs;
use std::path::PathBuf;

use pvm_alto::{default_options, execute_tx_fs, FsTxConfig};
use pvm_host::HostContext;
use pvm_runtime::{ContinuationOptions, DeterminismOptions};
use rustpython_vm::vm::ContinuationMode;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut trace_path: Option<String> = None;
    let mut trace_allow_all = false;
    let mut script_path: Option<String> = None;
    let mut input: Option<Vec<u8>> = None;
    let mut deterministic = true;
    let mut continuation_mode: Option<ContinuationMode> = None;
    let mut resume_bytes: Option<Vec<u8>> = None;
    let mut resume_key: Option<Vec<u8>> = None;
    let mut checkpoint_key: Option<Vec<u8>> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--trace-imports" => {
                let value = args.next().ok_or_else(|| usage())?;
                trace_path = Some(value);
            }
            "--trace-allow-all" => {
                trace_allow_all = true;
            }
            "--nondeterministic" => {
                deterministic = false;
            }
            "--continuation" => {
                let value = args.next().ok_or_else(|| usage())?;
                continuation_mode = Some(parse_continuation_mode(&value)?);
            }
            "--resume-bytes" => {
                let value = args.next().ok_or_else(|| usage())?;
                resume_bytes = Some(parse_hex_arg(&value)?);
            }
            "--resume-key" => {
                let value = args.next().ok_or_else(|| usage())?;
                resume_key = Some(parse_hex_arg(&value)?);
            }
            "--checkpoint-key" => {
                let value = args.next().ok_or_else(|| usage())?;
                checkpoint_key = Some(parse_hex_arg(&value)?);
            }
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(());
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--continuation=") {
                    continuation_mode = Some(parse_continuation_mode(value)?);
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--trace-imports=") {
                    trace_path = Some(value.to_owned());
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--resume-bytes=") {
                    resume_bytes = Some(parse_hex_arg(value)?);
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--resume-key=") {
                    resume_key = Some(parse_hex_arg(value)?);
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--checkpoint-key=") {
                    checkpoint_key = Some(parse_hex_arg(value)?);
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
        actor_addr: b"demo_actor".to_vec(),
        msg_id: Vec::new(),
        nonce: 0,
    };

    let config = FsTxConfig {
        state_dir: PathBuf::from("tmp/pvm_state"),
        events_path: PathBuf::from("tmp/pvm_events.log"),
        gas_limit: 1_000_000,
        context: ctx,
    };

    let mut options = default_options().with_source_path(script_path);
    if continuation_mode.is_some()
        || resume_bytes.is_some()
        || resume_key.is_some()
        || checkpoint_key.is_some()
    {
        let mode = continuation_mode.unwrap_or_else(|| {
            if resume_bytes.is_some() || resume_key.is_some() || checkpoint_key.is_some() {
                ContinuationMode::Checkpoint
            } else {
                ContinuationMode::Fsm
            }
        });
        options.continuation = Some(ContinuationOptions {
            mode,
            resume_bytes,
            resume_key,
            checkpoint_key,
        });
    }
    if !deterministic {
        options.deterministic = false;
        options.determinism = None;
    }
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

fn parse_continuation_mode(value: &str) -> Result<ContinuationMode, std::io::Error> {
    match value {
        "fsm" => Ok(ContinuationMode::Fsm),
        "checkpoint" => Ok(ContinuationMode::Checkpoint),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "continuation mode must be fsm or checkpoint",
        )),
    }
}

fn parse_hex_arg(value: &str) -> Result<Vec<u8>, std::io::Error> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.len() % 2 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "hex string must have even length",
        ));
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for idx in (0..bytes.len()).step_by(2) {
        let chunk = std::str::from_utf8(&bytes[idx..idx + 2]).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid hex string")
        })?;
        let byte = u8::from_str_radix(chunk, 16).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid hex string")
        })?;
        out.push(byte);
    }
    Ok(out)
}

fn usage() -> &'static str {
    "usage: pvm_runtime_chain_demo [--trace-imports <path>] [--trace-allow-all] [--nondeterministic] [--continuation fsm|checkpoint] [--resume-bytes <hex>] [--resume-key <hex>] [--checkpoint-key <hex>] <script.py> [input]"
}
