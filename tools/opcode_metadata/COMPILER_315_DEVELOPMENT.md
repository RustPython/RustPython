# Final 3.15 compiler ABI reconstruction

This is new work based on public RustPython commit
`06007c81fa99e813cbf4583dab326764ba0ef017`. Earlier local checkpoints were lost
with their execution environment. Their historical results are not validation
of this reconstruction.

The source target is CPython v3.15.0,
`d220409a13b77de7a6c30abe16ea90db01528edc`. `target_source.json` pins the
generator dependencies and directly consulted ABI sources. The active metadata
generator verifies those hashes before importing the CPython generator modules.
The target generator directory must also be free of bytecode caches; the gate
rejects existing pycs and disables writes before importing from the verified path.
`verify_stack_contract.py` compiles the pinned C metadata independently and
checks 3,430 argument-specific stack rows; Rust tests compare every supported
opcode against those rows. Unsupported specialized instructions are not claimed
implemented merely because their reference rows are recorded.

## Private cache contract

The intermediate ABI uses magic `0xDA16` (bytes `16 da 0d 0a`) and cache tag
`rustpython-315-devabi2`. Available repository and pinned-source definitions were
checked for collisions; this is not a globally reserved token. The lost
`0xDA15` development format is deliberately rejected, as are public CPython
formats. Change both token and tag whenever the intermediate format changes
incompatibly.

The shared Rust magic feeds importlib, source and sourceless caches, ZIP imports,
explicit `py_compile`/`compileall` writers, and direct Rust pyc readers. `-B`
does not prohibit cache reads or explicit writes. Validation uses a fresh owned
Lib copy, records cache inventories, and checks foreign timestamp/hash caches
and ZIP fallback/rejection. The CLI prepends `BUILDTIME_RUSTPYTHONPATH` ahead of
`RUSTPYTHONPATH`; a new environment path alone does not isolate cache reads.
Strict captures must verify actual module paths and keep their embedded Lib copy
separate from cache-writing Rust embedding tests. Headerless marshal and explicit frozen-data input
remain caller-version-coupled. Frozen modules must be rebuilt using the same
compiler and VM target; private pyc magic does not validate their separate
compressed format.

## Output and integration gates

The strict harness captures untouched marshal version 6 bytes before inspecting
code fields. It compares magic, marshal and recursively captured fields without
normalizing byte streams, exception ranges, reference flags, or nested code.
An empty corpus is an error. Its finite fixtures are witnesses, not a general
compiler-equivalence proof. Behavioral regressions belong in Python snippets;
Rust tests additionally check structural compiler and ownership invariants.
The reference invocation requires an empty `PYTHONPYCACHEPREFIX` so existing
certified-build caches are not read. The harness verifies that directory remains
empty and records runtime paths, cache settings and already-imported module
origins after serialization and field capture. These diagnostics are provenance;
they do not alter the three strict comparison domains.

Canonical `co_code` uses immutable prepublication bytes and a stable bytes owner;
the common marshal writer uses that actual field rather than live monitoring
instructions. The adaptive getter projects private Rust `LOAD_CONST` IDs220/221
to public `LOAD_CONST` at immutable canonical instruction boundaries, preserving
arguments, public specialization IDs and cache payload bytes. Legacy private
ID219 has no equivalent one-unit public encoding and raises `ValueError` on
adaptive export. Length and truncated canonical cache spans are checked; arbitrary
same-length postpublication embedding mutation remains outside the finalized-code
contract. Per-cell atomic reads do not establish a coherent concurrent snapshot
or repair inherited shared-storage accessor and mixed-width dispatch concerns.
The projection neither mutates executable instructions nor changes canonical or
marshal bytes. Runtime specialization choices need not match CPython.
Retained private `GET_YIELD_FROM_ITER` must never be emitted by the compiler.
The current local tree includes the separately reviewed builtin `frozendict`
prerequisite and exact final `Lib/opcode.py`, `Lib/_opcode_metadata.py`, and
`Lib/dis.py` imports made only through `scripts/update_lib`. The eight original
dis tests pass with certified final expectations. This local validation does not
waive Rust-first merge sequencing or establish a distributable implementation.
Future Lib updates must also use `scripts/update_lib` with the exact source pin;
manual copying or editing these files is not permitted.
The public magic transition requires full serialization compatibility,
reader/writer/frozen agreement, and the separately planned exact Python update.

Marshal reference ownership, compiler constant-origin chronology and mortal
interning remain separate work. No count offsets, flag masking, equal-text
over-interning or weakening of static-reference invariants is permitted.
The filename/surrogate family remains outside this stage.

## Public native opcode metadata

`generate_public_metadata.py --source <pinned-source>` generates the native
`_opcode` introspection domain from final C descriptors, deoptimization mapping
and stack expressions. Its 267-slot descriptor extent and 256-slot deopt extent
are deliberately separate from the executable Rust opcode enum. This includes
CPython-only specialized/tier2 descriptors and excludes private Rust IDs219–221;
it does not implement their execution or change cache magic. Specialized stack
effects remain invalid. Formula parsing fails closed and uses wrapping signed
32-bit arithmetic rather than panic-prone Rust arithmetic. This is an explicit
implementation policy, not a portable claim about C signed overflow; equivalence
outside the tested argument window remains unclaimed.

The bounded Python regression retains all514 numeric flag rows and a complete
10,794-row stack-result digest from the certified interpreter, including errors.
It uses ordinary nonnegative arguments and exact None/False/True jump values.
The numeric metadata matrix does not cover calling conversions; the separate
calling protocol regression below does not claim all diagnostic wording parity.
Canonical metadata/dis files are exact final imports through update_lib. Their
local integration does not waive Rust-first merge order or publication gates.

The local `_opcode` calling-conversion follow-on accepts index objects after
argument binding, converts the opcode before its operand, treats an explicit
`None` operand as zero, and requires exact `None`/`True`/`False` jump identities.
C-int and C-long overflow messages are local to this module. C-long narrowing
uses a panic-free low-bit policy for supported two's-complement target ABIs;
Linux oracle evidence does not certify a Windows run or C signed-overflow
formulas. Shared argument-binder and invalid-index-result diagnostic wording
remain separately visible in the complete calling matrix.
