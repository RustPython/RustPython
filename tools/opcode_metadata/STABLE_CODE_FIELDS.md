# Stable code fields and common writer routing

The storage stage follows canonical code storage commit6746df059. The subsequent
common-writer stage is described below. The subsequent actual-owner selector is described below. Native call ownership,
permanent string interning and filename handling remain unchanged.
Full serialization parity remains unresolved.

Runtime `Constants<Literal>` owns one immutable Python tuple. It does not keep a
second boxed list of owning child references beside a getter cache. The compiler
representation remains boxed. Cloning a runtime constant pool shares the tuple;
mutable access copies the elements before replacing the owner. Published tuples
are never mutated. The owner-bound Literal slice uses the existing transparent
layout over PyObjectRef and never calls the slice/frozenset constant adapter.

Surviving names, locals-plus names/kinds and line/exception tables have stable
Python owners. Derived variable/cell/free-name tuples are separate lazy caches.
Constructor names are copied after validation; replace retains unchanged names.
Public marshal decode retains actual field objects when normalized name member
identities permit it, otherwise uses a safe normalized copy. That copy has a
known tuple-identity difference from CPython's in-place name normalization.
Existing constant interning behavior is preserved, not expanded.

Decoder input co_code bytes are not retained as the exported co_code object.
That export remains A0's independent lazy cache of canonical normalized bytes.
The new reader hook defaults to the existing bytes/constants/code hook chain.
Runtime decoding drops temporary child vectors and replaced boxes before code
publication. Reader allow_code, type, depth and incomplete-reference checks stay
in force.

## Rust source API migration

- `PyCode::new(code, &Context)` now requires the creating runtime's context so
  final tuple/metadata objects exist before publication. Callers must still pass
  pristine instruction streams, as required by A0.
- Implementations of compiler-core `Constant` supply `TupleOwner: Clone +
  AsRef<[Self]>`. BasicBag uses boxed constants; the runtime uses a private
  validated tuple owner. Downstream bags must provide an immutable owner-bound
  element view. No Python object type is introduced into compiler-core.
- Public getters for co_linetable/co_exceptiontable no longer require a VM
  argument; they return retained Python bytes owners.
- A new `MarshalBag::make_code_with_fields` is additive. Its default delegates
  `make_code_with_constants_and_bytes`, preserving earlier customization hooks.

The checked-in Python test retains full certified streams in both alias orders,
checks identity and execution, and explicitly preserves the normalization gap.
Rust tests verify actual child-owner counts, COW isolation, publication and
reader-hook ordering. No test count is used by production serialization.

## Integration limits

The existing untracked PyCode policy and permanent-intern ownership contracts
are unchanged. Pinned CPython only tracks code objects in Py_GIL_DISABLED
builds; the certified GIL-enabled oracle is untracked too. This stage does not claim a general cycle-collection repair or mortal
interning parity. Python child ownership moves to the tuple edge and ordinary
Rust drops release it; a future traversal change must account for that real edge.
Compilation against current main after #9080 is a separate integration gate even
though the source audit found no direct overlap and transparent ownership remains.
Full workspace/CAPI/release and the canonical metadata dependencies remain
required before a general integration/CI-green claim.

## Common writer routing follow-on

The runtime marshal writer routes nine actual surviving code-field objects
through its ordinary object writer: canonical code bytes, constants, names,
localsplus names and kinds, name, qualname, line table and exception table.
The field callback runs before the compiler/frozen fallback computes any bytes
or temporary names. The filename operation stays unchanged in its original
position. Default compiler/frozen callers retain their previous full encoding.

Runtime constants are serialized as the real tuple, including its normal child
depth, rather than a synthetic tuple header followed by flattened children.
The canonical code getter's real owner lives through the complete code write;
other fields are borrowed from their existing owners. Each reference-table
reservation additionally owns its referent until that per-call table drops,
including set-sort temporary tables and error exits. No dictionary lock spans
recursive serialization.

This changes ordinary code-field bytes and fixes the live-monitoring export
source used by this writer. At the routing checkpoint, reference selection was
still blanket; the selector follow-on below replaces that rule. Direct, named
and aliased roots retain their real native-call owners;
no counts or flags are adjusted to imitate CPython. Filename identity, interning
lifetime and native argument provenance remain separate compatibility gaps.
Storage and writer changes alone do not establish complete marshal parity or
public cache-format compatibility.


## Observational actual-owner reference selection

Before reference lookup or reservation, the common writer observes the actual
strong count. A unique object is emitted without registration unless it is an
exact interned string. One decision controls lookup, reservation, flag emission
and completion. Table owners, canonical getter owners, dictionary snapshots and
set-sort temporary owners remain real and unchanged; no counts are subtracted.

The relaxed count observation changes only wire registration. It does not prove
exclusive access or authorize borrowing or mutation. Concurrent weak acquisition
or graph changes can affect bytes and decoded alias observations; serialization
is not a snapshot or a claim of CPython free-threaded uniqueness parity. Existing
immortal and permanent-interner contracts remain intact. Native direct/named
root differences and compiler-origin sharing remain separate parity gaps.

## Compilation-local materialization (A4)

Source/AST and Basic CodeObject publication share one private pool of typed,
owner-free Rust keys. The five surviving fields are finalized in this order:
exception table, line table, names, constants, localsplusnames. Nested code values
are finalized before their PyRefs enter the private root graph; the pool drops
before root publication, including on unwinding. Exported co_code, localspluskinds
and separately lazy var/cell/free tuples are not pooled.

The additive IntoCodeObject::into_pycode method defaults to the former runtime
construction path. Its Basic override joins the source path; downstream
implementations inherit the default without a new required method. Raw Basic
into_code_object shares constants and nested bodies but cannot promise finalized
root field identities. Frozen decoding, runtime Literal input, public readers,
constructors and replace retain their prior routes.

Keys distinguish types and all signed zeros. Frozensets are keyed by their actual
surviving element types, with temporary snapshot owners released before cache
selection. No leaking Literal borrow adapter is used to inspect runtime keys.
NaNs and aggregates containing them, code objects and code-containing aggregates,
and entire slice subtrees remain excluded. Noncanonical equal-text name tuple
winners are rejected without mutating them; the normalized-copy identity gap
remains. No global interning policy or native transport changes are made.

This bottom-up optimized-graph materialization does not reproduce CPython's
parent-first recursive registration or earlier compiler/assembly first winners,
including discarded constants and input bytecode. Exact chronological sharing
and full marshal parity remain unresolved; finite passing witnesses cannot prove
those properties. Real pool owners are removed by ordinary drop, not count
adjustments. The runtime's existing GC policy remains unchanged.
