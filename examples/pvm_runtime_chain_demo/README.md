# PVM Runtime Chain Demo (Filesystem Host)

This demo simulates an Alto-style chain host with a filesystem-backed `HostApi`.
It runs a Python contract through `pvm-runtime` and writes state/events to local files.

## Files

- `main.rs`: Example runner that loads the Python contract and executes it.
- `contract.py`: Sample contract using `pvm_host`.
- `determinism_demo.py`: Determinism demo (import guard + stdlib shims + host context).
- `escrow_marketplace_demo.py`: Escrow marketplace flow with expiry, funding, and release.
- `batch_payroll_demo.py`: Batch payroll settlement with fees and audit sampling.
- `staking_rewards_demo.py`: Staking rewards distribution with weighted proposer selection.

## Run

From the repo root:

```bash
cargo run --release --example pvm_runtime_chain_demo -- examples/pvm_runtime_chain_demo/contract.py hello
```

On macOS with Homebrew libffi, you may need:

```bash
DYLD_LIBRARY_PATH=/opt/homebrew/opt/libffi/lib \
cargo run --release --example pvm_runtime_chain_demo -- examples/pvm_runtime_chain_demo/contract.py hello
```

## Output and Artifacts

- `output_hex=...` printed to stdout (hex-encoded bytes returned by the contract).
- State files in `tmp/pvm_state/` (keyed by hex-encoded keys).
- Event log in `tmp/pvm_events.log` (one line per event: `topic:hex_payload`).

## Determinism Demo

```bash
DYLD_LIBRARY_PATH=/opt/homebrew/opt/libffi/lib \
cargo run --release --example pvm_runtime_chain_demo -- examples/pvm_runtime_chain_demo/determinism_demo.py hello
```

The contract output is JSON (hex-encoded on stdout) with:

- Deterministic time and randomness via `time`/`random` stdlib shims.
- Blocked modules and file IO recorded under `blocked`.
- Host context echoed back (hashes and sender as hex).

Run the command multiple times and compare `output_hex` for identical results.

## Determinism Check (Multi-run)

```bash
python examples/pvm_runtime_chain_demo/determinism_check.py --runs 5 --decode
```

Use `--keep-state` if you want to keep `tmp/pvm_state` between runs.

## Business Scenario Demos

Each demo supports `demo` as input for a built-in batch, or a JSON object with
`action`/`params` or `actions` (list). Outputs include a `state_hash` to
compare determinism.

```bash
python examples/pvm_runtime_chain_demo/determinism_check.py \
  --runs 5 --decode \
  --script examples/pvm_runtime_chain_demo/escrow_marketplace_demo.py \
  --input demo
```

```bash
python examples/pvm_runtime_chain_demo/determinism_check.py \
  --runs 5 --decode \
  --script examples/pvm_runtime_chain_demo/batch_payroll_demo.py \
  --input demo
```

```bash
python examples/pvm_runtime_chain_demo/determinism_check.py \
  --runs 5 --decode \
  --script examples/pvm_runtime_chain_demo/staking_rewards_demo.py \
  --input demo
```

## Import Trace (Whitelist Generator)

Generate an import trace (with non-whitelisted imports allowed) and print a
suggested whitelist:

```bash
python examples/pvm_runtime_chain_demo/determinism_check.py \
  --runs 1 \
  --trace-imports tmp/pvm_import_trace.json \
  --trace-allow-all \
  --print-whitelist
```

Or run the binary directly:

```bash
DYLD_LIBRARY_PATH=/opt/homebrew/opt/libffi/lib \
cargo run --release --example pvm_runtime_chain_demo -- \
  --trace-imports tmp/pvm_import_trace.json \
  --trace-allow-all \
  examples/pvm_runtime_chain_demo/determinism_demo.py hello
```

## Contract Behavior

- Reads and increments a `counter` state key.
- Emits a `demo` event.
- Returns `b"ok:<input>:h=<block_height>"`.
