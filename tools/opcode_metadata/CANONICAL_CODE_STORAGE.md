# Canonical code storage follow-on

This storage-only follow-on starts from reviewed checkpoint a974ec938d700459d5badd578df59b587f4a4396.
It does not change the common marshal writer, reference policy, filename handling,
private magic, native argument transport, or tuple/metadata ownership.

Pinned Objects/codeobject.c:_PyCode_GetCode caches a stable bytes object and
uses monitoring-aware deoptimization; code_replace_impl obtains that public
representation when co_code is omitted. Rust's previous live original_bytes()
export cannot recover generic INSTRUMENTED_LINE/INSTRUCTION side-table opcodes.

PyCode::new now captures immutable canonical bytes immediately after GET_ITER
normalization, before code publication. co_code lazily creates one PyBytes object
from that backing through the threading-aware OnceCell. Python replace without
co_code parses the immutable backing instead of cloning live execution units.
Source/AST, Basic bytecode materialization, Python constructor/replace, frozen
code and the public marshal reader all converge on PyCode::new. No context API
change is needed for this narrow lazy bytes cache.

The public embedding constructor requires pristine input. Copying a monitored
PyCode.code into a new PyCode cannot carry its dynamic monitoring side tables.
Logical post-finalization code mutation is unsupported; execution specialization
and instrumentation may mutate live instructions but cannot replace canonical
public bytes. Embedders replacing logical code must construct a new PyCode from
canonical bytes. This is a documented API contract, not an unsafe lifetime claim.

The additive Python regression checks cold/warmed stable identity, constructor,
replace, public decoding and a core frozen code object. Independent compilations
provide expected bytes so each monitored target's first getter call occurs inside
an active LINE or INSTRUCTION callback. Replacement construction and execution
there must never copy live monitoring wrappers. All filenames are fixed ASCII.

Crucially, marshal's current serialize_code_with still reads live instructions.
This getter/storage change alone does not fix marshal-under-monitoring or make
full streams exact. A later separately reviewed common-writer connection must
use the actual cached object and preserve its temporary owner through all fields.
No public serialization compatibility or full-byte parity is claimed here.
