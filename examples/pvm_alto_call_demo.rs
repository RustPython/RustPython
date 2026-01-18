use std::fs;
use std::path::PathBuf;

use pvm_alto::{default_options, execute_tx_fs, FsTxConfig};
use pvm_host::HostContext;

fn exec_tx(
    code: &[u8],
    input: &str,
    sender: &[u8],
    state_dir: &PathBuf,
    events_path: &PathBuf,
    script_path: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let ctx = HostContext {
        block_height: 1,
        block_hash: [0u8; 32],
        tx_hash: [1u8; 32],
        sender: sender.to_vec(),
        timestamp_ms: 1_700_000_000_000,
        actor_addr: b"demo_actor".to_vec(),
        msg_id: Vec::new(),
        nonce: 0,
    };

    let config = FsTxConfig {
        state_dir: state_dir.clone(),
        events_path: events_path.clone(),
        gas_limit: 1_000_000,
        context: ctx,
    };

    let options = default_options().with_source_path(script_path);
    Ok(execute_tx_fs(code, input.as_bytes(), config, &options)?)
}

fn print_output(label: &str, output: &[u8]) {
    println!("{}: {}", label, String::from_utf8_lossy(output));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script_path = "examples/pvm_actor_transfer_demo/contract.py";
    let code = fs::read(script_path)?;

    let state_dir = PathBuf::from("tmp/pvm_alto_call_state");
    let events_path = PathBuf::from("tmp/pvm_alto_call_events.log");

    let init = r#"{"action":"init","params":{"balances":{"alice":1000,"bob":500}}}"#;
    let out = exec_tx(
        &code,
        init,
        b"alice",
        &state_dir,
        &events_path,
        script_path,
    )?;
    print_output("init", &out);

    let transfer = r#"{"action":"transfer","params":{"to":"bob","amount":150}}"#;
    let out = exec_tx(
        &code,
        transfer,
        b"alice",
        &state_dir,
        &events_path,
        script_path,
    )?;
    print_output("transfer", &out);

    let balance = r#"{"action":"balance","params":{"user":"bob"}}"#;
    let out = exec_tx(
        &code,
        balance,
        b"bob",
        &state_dir,
        &events_path,
        script_path,
    )?;
    print_output("balance", &out);

    Ok(())
}
