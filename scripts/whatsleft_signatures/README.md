# Signature observation regression gate

CI reuses the existing RustPython executable and single `whats_left.py` scan.
It runs immediately after the build, before the expensive CPython test suite.
It checks raw native `__text_signature__` separately from `inspect.signature`,
and rejects new/changed known failures, lost or changed selected scopes, and
resolved exceptions left in the baseline. The last rule prevents a later PR
from reintroducing a previously fixed mismatch under an obsolete exception.
Only process addresses in exception-shaped repr diagnostics are normalized.
Raw signature text, parameter names and default literals are never normalized.
When CPython has no raw signature (`None` or an absent attribute), an additional
RustPython signature is allowed without a baseline exception. Its value may
change without claiming CPython parity; the observed path stays in coverage.
This is asymmetric: a missing RustPython signature where CPython provides one
still fails. Unknown observations and differing contexts are not exempted.
No runtime call compatibility is inferred from either signature representation.
Type-document observations are not constructor compatibility evidence.

## Scope and known limitations

The scanner selects builtin/iterator objects, top-level modules and selected
native class members. It does not enumerate every possible object, nested
package, instance or inherited API surface. Unknown/unrepresentative raw
observations and CPython introspection/import failures remain explicitly
unverified. A passing gate means no change against reviewed observations, not
zero mismatches or complete CPython compatibility.

The baselines are reviewed platform-specific CI observations. The initial
run [42421287760](https://github.com/RustPython/RustPython/actions/runs/42421287760)
is superseded where recorded by
[42491705325](https://github.com/RustPython/RustPython/actions/runs/42491705325),
head `4a6ddde862e8c0e4082f1434da0c9b3e2fb609b7`, merge
`3b4735987efcc7ba59d454a1f6cf9d667a631778`. Each baseline records its own provenance.
They use CPython **3.15.0**, not the
separate local 3.15.1 inventory. Their exact OS, architecture, reference version
and RustPython feature context must match. CI pins the oracle using
`.python-version`. Platform-specific paths are not suppressed on other OSes.

`observations` groups identical failures for size, but every exception names
individual paths and exact observed values. `coverage` hashes sorted paths in
each selected scope, including selection decisions. Changes (including new
coverage) require review; a hash is not a claim of full API coverage. Full
readable data and host context remain CI artifacts, including on gate failure.

## Updating a baseline

1. Read the failing artifact and compare its context with the baseline.
2. Review every changed signature and observation scope against CPython 3.15.
   A disappeared API, failed import or new unknown is not a signature fix.
3. Recreate only the corresponding platform file from that verified artifact:
   ```sh
   python scripts/check_whatsleft_signatures.py whats-left.json \
     scripts/whatsleft_signatures/Linux.json --context '<exact context>' \
     --write-baseline
   ```
4. Inspect the diff. Remove resolved exceptions in the same PR as each fix;
   do not retain them as permission for future regressions. Preserve/update
   the provenance and state why any newly accepted observation is necessary.
5. Run the checker without `--write-baseline` and the focused tests:
   ```sh
   python -m unittest discover -s scripts/tests -p 'test_whatsleft*.py'
   ```

CI never writes its own baseline and never accepts artifacts from another
platform as proof. Updating the oracle/build features also requires reviewing
all affected baselines. The intended end state is no comparable signature
exceptions, while unsupported/unobservable coverage stays separately visible.
