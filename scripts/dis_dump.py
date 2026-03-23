#!/usr/bin/env python3
"""Dump normalized bytecode for Python source files as JSON.

Designed to produce comparable output across different Python implementations.
Normalizes away implementation-specific details (byte offsets, memory addresses)
while preserving semantic instruction content.

Usage:
    python dis_dump.py Lib/
    python dis_dump.py path/to/file.py
"""

import dis
import json
import os
import re
import sys
import types

# Non-semantic filler instructions to skip
SKIP_OPS = frozenset({"CACHE", "PRECALL"})

# Opname normalization: map variant instructions to their base form.
# These variants differ only in optimization hints, not semantics.
_OPNAME_NORMALIZE = {
    "LOAD_FAST_BORROW": "LOAD_FAST",
    "LOAD_FAST_BORROW_LOAD_FAST_BORROW": "LOAD_FAST_LOAD_FAST",
    "LOAD_FAST_CHECK": "LOAD_FAST",
    "JUMP_BACKWARD_NO_INTERRUPT": "JUMP_BACKWARD",
}

# Jump instruction names (fallback when hasjrel/hasjabs is incomplete)
_JUMP_OPNAMES = frozenset({
    "JUMP", "JUMP_FORWARD", "JUMP_BACKWARD", "JUMP_BACKWARD_NO_INTERRUPT",
    "POP_JUMP_IF_TRUE", "POP_JUMP_IF_FALSE",
    "POP_JUMP_IF_NONE", "POP_JUMP_IF_NOT_NONE",
    "JUMP_IF_TRUE_OR_POP", "JUMP_IF_FALSE_OR_POP",
    "FOR_ITER", "SEND",
})

_JUMP_OPCODES = None


def _jump_opcodes():
    global _JUMP_OPCODES
    if _JUMP_OPCODES is None:
        _JUMP_OPCODES = set()
        if hasattr(dis, "hasjrel"):
            _JUMP_OPCODES.update(dis.hasjrel)
        if hasattr(dis, "hasjabs"):
            _JUMP_OPCODES.update(dis.hasjabs)
    return _JUMP_OPCODES


def _is_jump(inst):
    """Check if an instruction is a jump (by opcode set or name)."""
    return inst.opcode in _jump_opcodes() or inst.opname in _JUMP_OPNAMES


def _normalize_argrepr(argrepr):
    """Strip runtime-specific details from arg repr."""
    if argrepr.startswith("<code object "):
        # Extract just the name, stripping address and file/line info.
        # Formats seen across interpreters:
        #   <code object foo at 0xADDR, file "x.py", line 1>  (CPython 3.14)
        #   <code object foo at 0xADDR>                        (RustPython)
        name = argrepr[len("<code object "):]
        for marker in (" at 0x", ", file ", " file "):
            idx = name.find(marker)
            if idx >= 0:
                name = name[:idx]
        return "<code object %s>" % name.rstrip(">").strip()
    # Normalize COMPARE_OP: strip bool(...) wrapper from CPython 3.14
    # e.g. "bool(==)" -> "==", "bool(<)" -> "<"
    m = re.match(r"^bool\((.+)\)$", argrepr)
    if m:
        return m.group(1)
    # Remove memory addresses from other reprs
    argrepr = re.sub(r" at 0x[0-9a-fA-F]+", "", argrepr)
    # Remove LOAD_ATTR/LOAD_SUPER_ATTR suffixes: " + NULL|self", " + NULL"
    argrepr = re.sub(r" \+ NULL\|self$", "", argrepr)
    argrepr = re.sub(r" \+ NULL$", "", argrepr)
    return argrepr


_IS_RUSTPYTHON = hasattr(sys, "implementation") and sys.implementation.name == "rustpython"

# RustPython's ComparisonOperator enum values → operator strings
_RP_CMP_OPS = {0: "<", 1: "<", 2: ">", 3: "!=", 4: "==", 5: "<=", 6: ">="}


def _resolve_arg_fallback(code, opname, arg):
    """Resolve a raw argument to its human-readable form.

    Used when the dis module doesn't populate argrepr (e.g., on RustPython).
    """
    if not isinstance(arg, int):
        return arg
    try:
        if "FAST" in opname:
            if 0 <= arg < len(code.co_varnames):
                return code.co_varnames[arg]
        elif opname == "LOAD_CONST":
            if 0 <= arg < len(code.co_consts):
                return _normalize_argrepr(repr(code.co_consts[arg]))
        elif opname in ("LOAD_DEREF", "STORE_DEREF", "DELETE_DEREF",
                        "LOAD_CLOSURE", "MAKE_CELL", "COPY_FREE_VARS"):
            # These use fastlocal index: nlocals + cell/free offset
            nlocals = len(code.co_varnames)
            cell_and_free = code.co_cellvars + code.co_freevars
            cell_idx = arg - nlocals
            if 0 <= cell_idx < len(cell_and_free):
                return cell_and_free[cell_idx]
            elif 0 <= arg < len(cell_and_free):
                # Fallback: direct index into cell_and_free
                return cell_and_free[arg]
        elif opname in ("LOAD_NAME", "STORE_NAME", "DELETE_NAME",
                        "LOAD_GLOBAL", "STORE_GLOBAL", "DELETE_GLOBAL",
                        "LOAD_ATTR", "STORE_ATTR", "DELETE_ATTR",
                        "IMPORT_NAME", "IMPORT_FROM"):
            if 0 <= arg < len(code.co_names):
                return code.co_names[arg]
        elif opname == "LOAD_SUPER_ATTR":
            name_idx = arg >> 2
            if 0 <= name_idx < len(code.co_names):
                return code.co_names[name_idx]
    except Exception:
        pass
    return arg


def _extract_instructions(code):
    """Extract normalized instruction list from a code object.

    - Filters out CACHE/PRECALL instructions
    - Converts jump targets from byte offsets to instruction indices
    - Resolves argument names via fallback when argrepr is missing
    - Normalizes argument representations
    """
    try:
        raw = list(dis.get_instructions(code))
    except Exception as e:
        return [["ERROR", str(e)]]

    # Build filtered list and offset-to-index mapping
    filtered = []
    offset_to_idx = {}
    for inst in raw:
        if inst.opname in SKIP_OPS:
            continue
        offset_to_idx[inst.offset] = len(filtered)
        filtered.append(inst)

    # Map offsets that land on CACHE slots to the next real instruction
    for inst in raw:
        if inst.offset not in offset_to_idx:
            for fi, finst in enumerate(filtered):
                if finst.offset >= inst.offset:
                    offset_to_idx[inst.offset] = fi
                    break


    result = []
    for inst in filtered:
        opname = _OPNAME_NORMALIZE.get(inst.opname, inst.opname)
        if _is_jump(inst) and isinstance(inst.argval, int):
            target_idx = offset_to_idx.get(inst.argval)
            # If argval wasn't resolved (RustPython), compute target offset
            if target_idx is None and inst.arg is not None:
                if "FORWARD" in inst.opname:
                    target_off = inst.offset + 2 + inst.arg * 2
                    target_idx = offset_to_idx.get(target_off)
                elif "BACKWARD" in inst.opname:
                    # Try several cache sizes (0-3) for backward jumps
                    for cache in range(4):
                        target_off = inst.offset + 2 + cache * 2 - inst.arg * 2
                        if target_off >= 0 and target_off in offset_to_idx:
                            target_idx = offset_to_idx[target_off]
                            break
            if target_idx is None:
                target_idx = inst.argval
            result.append([opname, "->%d" % target_idx])
        elif inst.opname == "COMPARE_OP":
            # Normalize COMPARE_OP across interpreters (different encodings)
            if _IS_RUSTPYTHON:
                cmp_str = _RP_CMP_OPS.get(inst.arg, inst.argrepr)
            else:
                cmp_str = _normalize_argrepr(inst.argrepr) if inst.argrepr else str(inst.arg)
            result.append([opname, cmp_str])
        elif inst.arg is not None and inst.argrepr:
            result.append([opname, _normalize_argrepr(inst.argrepr)])
        elif inst.arg is not None:
            resolved = _resolve_arg_fallback(code, inst.opname, inst.arg)
            result.append([opname, resolved])
        else:
            result.append([opname])

    return result


def _dump_code(code):
    """Recursively dump a code object and its nested code objects."""
    name = getattr(code, "co_qualname", None) or code.co_name
    children = [_dump_code(c) for c in code.co_consts if isinstance(c, types.CodeType)]
    r = {"name": name, "insts": _extract_instructions(code)}
    if children:
        r["children"] = children
    return r


def process_file(path):
    """Compile a single file and return its bytecode dump."""
    try:
        with open(path, "rb") as f:
            source = f.read()
        code = compile(source, path, "exec")
        return {"status": "ok", "code": _dump_code(code)}
    except SyntaxError as e:
        return {"status": "error", "error": "%s (line %s)" % (e.msg, e.lineno)}
    except Exception as e:
        return {"status": "error", "error": str(e)}


def main():
    if len(sys.argv) < 2:
        sys.stderr.write("Usage: %s <path> [...]\n" % sys.argv[0])
        sys.exit(1)

    results = {}
    for target in sys.argv[1:]:
        if os.path.isdir(target):
            for root, dirs, files in os.walk(target):
                dirs[:] = sorted(
                    d for d in dirs if d != "__pycache__" and not d.startswith(".")
                )
                for fname in sorted(files):
                    if fname.endswith(".py"):
                        fpath = os.path.join(root, fname)
                        relpath = os.path.relpath(fpath, target)
                        results[relpath] = process_file(fpath)
        elif target.endswith(".py"):
            results[os.path.basename(target)] = process_file(target)

    json.dump(results, sys.stdout, ensure_ascii=False, separators=(",", ":"))


if __name__ == "__main__":
    main()
