# Signature regression gate

CI reuses the existing RustPython build and whatsleft scan. The gate checks
comparable raw native `__text_signature__` and `inspect.signature` separately.
It does not freeze unrelated import errors, unknowns, missing APIs, selection
decisions, or the complete discovery inventory. Those diagnostics remain in
the full scan artifact.

CPython must provide a signature to establish a disagreement. Additional
RustPython signatures are allowed without an exception. Raw comparisons require
matching native kind/binding contexts; a missing actual signature is retained
as a failure. Inspected comparisons likewise retain invalid actual signatures
as compact failures, without freezing exception diagnostics. They cover the scanner's
successfully compared API paths, not inherited differences whose actual
inspected value was not retained. Type-document text is not proof of constructor
or runtime call compatibility.

Each platform baseline contains only exact signature mismatches and four
fingerprints of the checked raw/inspected builtin/module scopes. Fingerprints
include API paths and reference signatures. A previously checked signature
becoming absent, unobservable, or uncomparable changes its fingerprint and fails;
it cannot silently count as a fix. New comparable paths and reference changes
also require review. This protects the checked surface, not all possible APIs.
Truncated, ambiguous or empty reference inventories are rejected.

The baseline context pins OS, architecture, CPython version and build features.
The source artifacts use CPython **3.15.0**, not the local 3.15.1 test oracle.
Their recorded provenance is CI run
[42491705325](https://github.com/RustPython/RustPython/actions/runs/42491705325),
head `4a6ddde862e8c0e4082f1434da0c9b3e2fb609b7`.
They were reclassified with this narrower checker; no runtime improvements are
implied by removing unrelated diagnostic exceptions. Four buffer-constructor
exceptions per platform were also removed after #9744 merged into current main.
That removal is based on the shared implementation and a verified Linux binary;
the next hosted run must confirm all three current platform baselines.

## Updating a baseline

1. Read the failing full scan/context artifact and compare with the previous
   artifact. Review changed checked paths and signatures against CPython 3.15.
   A lost observation is not evidence of compatibility.
2. Recreate only the matching platform baseline:
   ```sh
   python scripts/check_whatsleft_signatures.py whats-left.json \
     scripts/whatsleft_signatures/Linux.json --context '<exact context>' \
     --write-baseline
   ```
3. Review the diff and preserve provenance. Remove resolved exceptions in the
   same PR as the fix; stale exceptions must not permit a later regression.
4. Run the checker without `--write-baseline` and:
   ```sh
   python -m unittest discover -s scripts/tests -p 'test_whatsleft*.py'
   ```

CI never writes its own baseline. Passing means no regression in the checked
signatures, not zero mismatches or complete CPython compatibility.
