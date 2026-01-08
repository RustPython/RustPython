# PVM DEX Demo (Filesystem Host)

This demo shows a tiny constant-product DEX contract running in `pvm-runtime`
with a filesystem-backed host.

## Files

- `main.rs`: Example runner.
- `contract.py`: DEX contract.

## Run

From the repo root:
DYLD_LIBRARY_PATH=/opt/homebrew/opt/libffi/lib \
```bash
cargo run --release --example pvm_dex_demo -- examples/pvm_dex_demo/contract.py \
  '{"action":"init","params":{"token_a":"USDC","token_b":"ETH","fee_bps":30}}'
```

Mint balances (faucet-style):

```bash
cargo run --release --example pvm_dex_demo -- --sender alice examples/pvm_dex_demo/contract.py \
  '{"action":"mint","params":{"token":"USDC","amount":100000}}'

cargo run --release --example pvm_dex_demo -- --sender alice examples/pvm_dex_demo/contract.py \
  '{"action":"mint","params":{"token":"ETH","amount":100}}'
```

Add liquidity:

```bash
cargo run --release --example pvm_dex_demo -- --sender alice examples/pvm_dex_demo/contract.py \
  '{"action":"add_liquidity","params":{"amount_a":50000,"amount_b":50}}'
```

Swap:

```bash
cargo run --release --example pvm_dex_demo -- --sender bob examples/pvm_dex_demo/contract.py \
  '{"action":"mint","params":{"token":"USDC","amount":1000}}'

cargo run --release --example pvm_dex_demo -- --sender bob examples/pvm_dex_demo/contract.py \
  '{"action":"swap","params":{"token_in":"USDC","amount_in":1000,"min_out":1}}'
```

Check balances and pool:

```bash
cargo run --release --example pvm_dex_demo -- --sender bob examples/pvm_dex_demo/contract.py \
  '{"action":"balance"}'

cargo run --release --example pvm_dex_demo -- examples/pvm_dex_demo/contract.py \
  '{"action":"info"}'
```

## Output

The runner prints `output_hex=...`. Decode with:

```bash
python - <<'PY'
hex_str = "PASTE_OUTPUT_HEX"
print(bytes.fromhex(hex_str).decode("utf-8"))
PY
```

## State and events

- State stored in `tmp/pvm_dex_state/`.
- Events appended to `tmp/pvm_dex_events.log`.
- Delete those paths to reset the demo.

## Actions

- `init`: set `token_a`, `token_b`, `fee_bps`.
- `mint`: faucet-like balance top-up (`token`, `amount`).
- `add_liquidity`: add to pool (`amount_a`, `amount_b`).
- `remove_liquidity`: withdraw (`lp_amount`).
- `swap`: swap token (`token_in`, `amount_in`, `min_out`).
- `quote`: price estimate (`token_in`, `amount_in`).
- `balance`: user balances and LP.
- `info`: pool and config.
