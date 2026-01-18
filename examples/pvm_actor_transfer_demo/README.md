# PVM Actor Transfer Demo (Filesystem Host)

This demo shows a minimal actor-style contract with balances and transfers.

## Files

- `main.rs`: Example runner.
- `contract.py`: Actor transfer contract.

## Run

From the repo root:

```bash
cargo run --release --example pvm_actor_transfer_demo -- \
  examples/pvm_actor_transfer_demo/contract.py \
  '{"action":"init","params":{"balances":{"alice":1000,"bob":500}}}'
```

Mint to a user:

```bash
cargo run --release --example pvm_actor_transfer_demo -- \
  examples/pvm_actor_transfer_demo/contract.py \
  '{"action":"mint","params":{"to":"carol","amount":200}}'
```

Transfer from the sender:

```bash
cargo run --release --example pvm_actor_transfer_demo -- --sender alice \
  examples/pvm_actor_transfer_demo/contract.py \
  '{"action":"transfer","params":{"to":"bob","amount":150}}'
```

Check balance:

```bash
cargo run --release --example pvm_actor_transfer_demo -- --sender bob \
  examples/pvm_actor_transfer_demo/contract.py \
  '{"action":"balance"}'
```

## Alto call example

Programmatic call using `pvm_alto`:

```bash
cargo run --release --example pvm_alto_call_demo
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

- State stored in `tmp/pvm_actor_transfer_state/`.
- Events appended to `tmp/pvm_actor_transfer_events.log`.
- Delete those paths to reset the demo.

## Actions

- `init`: initialize balances (`balances` map).
- `mint`: add balance (`to`, `amount`).
- `transfer`: transfer from sender (`to`, `amount`).
- `balance`: read balance (`user`, default sender).
- `info`: dump full state.
