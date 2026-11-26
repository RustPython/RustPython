# Signature parity census

The desktop CI matrix reuses the RustPython executable from the ordinary
tests. After those tests, a separate step scans against **CPython 3.15.2**
and fails on every comparable raw `__text_signature__` or
`inspect.signature` mismatch. An earlier failed test does not suppress the
census if the build and reference setup succeeded. The three platform jobs
do not cancel each other on failure.

The `whats-left-*` artifacts preserve the full scan and runtime context
alongside the `faulthandler` keyword probe. The keyword probe runs in child
processes without raising signals, even when signature comparison fails.
It checks examples where CPython exposes a raw signature which
`inspect.signature` cannot parse.

A green census means zero mismatches in the **comparable** set, not that
every API has been observed. Review missing imports, uncomparable bindings,
inherited APIs and other coverage gaps in the artifacts before making this
a required check. Do not use a disappearing observation as evidence of a
fix. The historical 3.15.0 baselines are not inputs to this census.

To run the same check with an existing executable:

```sh
python -I scripts/whats_left.py --signature --json-output whats-left.json \
    --rustpython target/release/rustpython
python scripts/check_whatsleft_signatures.py whats-left.json --require-parity
python -I scripts/check_faulthandler_keywords.py target/release/rustpython
```
