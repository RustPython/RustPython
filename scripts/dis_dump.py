#!/usr/bin/env python3
"""Dump normalized bytecode for Python source files as JSON.

Designed to produce comparable output across different Python implementations.
Normalizes away implementation-specific details (byte offsets, memory addresses)
while preserving semantic instruction content.

Usage:
    python dis_dump.py Lib/
    python dis_dump.py --base-dir Lib path/to/file.py
"""

import argparse
import dis
import json
import os
import re
import sys
import types

# Raw bytecode parity mode: do not skip any instructions.
SKIP_OPS = frozenset()

_OPNAME_NORMALIZE = {}
_SUPER_DECOMPOSE = {}

# Jump instruction names (fallback when hasjrel/hasjabs is incomplete)
_JUMP_OPNAMES = frozenset(
    {
        "JUMP",
        "JUMP_FORWARD",
        "JUMP_BACKWARD",
        "JUMP_BACKWARD_NO_INTERRUPT",
        "POP_JUMP_IF_TRUE",
        "POP_JUMP_IF_FALSE",
        "POP_JUMP_IF_NONE",
        "POP_JUMP_IF_NOT_NONE",
        "JUMP_IF_TRUE_OR_POP",
        "JUMP_IF_FALSE_OR_POP",
        "FOR_ITER",
        "SEND",
    }
)

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
        name = argrepr[len("<code object ") :]
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

    # Normalize unicode escapes
    def _unescape(m):
        try:
            cp = int(m.group(1), 16)
            if 0xD800 <= cp <= 0xDFFF:
                return m.group(0)
            return chr(cp)
        except (ValueError, OverflowError):
            return m.group(0)

    argrepr = re.sub(r"\\u([0-9a-fA-F]{4})", _unescape, argrepr)
    argrepr = re.sub(r"\\U([0-9a-fA-F]{8})", _unescape, argrepr)
    return argrepr


_IS_RUSTPYTHON = (
    hasattr(sys, "implementation") and sys.implementation.name == "rustpython"
)

# RustPython's ComparisonOperator enum values → operator strings
_RP_CMP_OPS = {0: "<", 1: "<=", 2: "==", 3: "!=", 4: ">", 5: ">="}


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
        elif opname in (
            "LOAD_DEREF",
            "STORE_DEREF",
            "DELETE_DEREF",
            "LOAD_CLOSURE",
            "MAKE_CELL",
        ):
            # arg is localsplus index:
            #   0..nlocals-1 = varnames (parameter cells reuse these slots)
            #   nlocals.. = non-parameter cells + freevars
            nlocals = len(code.co_varnames)
            if arg < nlocals:
                return code.co_varnames[arg]
            varnames_set = set(code.co_varnames)
            nonparam_cells = [v for v in code.co_cellvars if v not in varnames_set]
            extra = nonparam_cells + list(code.co_freevars)
            idx = arg - nlocals
            if 0 <= idx < len(extra):
                return extra[idx]
        elif opname in (
            "LOAD_NAME",
            "STORE_NAME",
            "DELETE_NAME",
            "LOAD_GLOBAL",
            "STORE_GLOBAL",
            "DELETE_GLOBAL",
            "LOAD_ATTR",
            "STORE_ATTR",
            "DELETE_ATTR",
            "IMPORT_NAME",
            "IMPORT_FROM",
            "LOAD_FROM_DICT_OR_GLOBALS",
        ):
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

    # Build filtered list and offset-to-index mapping for the normalized stream.
    # This must use post-decomposition indices; otherwise a superinstruction that
    # expands into multiple logical ops shifts later jump targets by 1.
    filtered = []
    offset_to_idx = {}
    normalized_idx = 0
    for inst in raw:
        if inst.opname in SKIP_OPS:
            continue
        opname = _OPNAME_NORMALIZE.get(inst.opname, inst.opname)
        offset_to_idx[inst.offset] = normalized_idx
        normalized_idx += len(_SUPER_DECOMPOSE.get(opname, (opname,)))
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

        # Decompose superinstructions into individual ops
        if opname in _SUPER_DECOMPOSE:
            op1, op2 = _SUPER_DECOMPOSE[opname]
            if isinstance(inst.arg, int):
                idx1 = (inst.arg >> 4) & 0xF
                idx2 = inst.arg & 0xF
            else:
                idx1, idx2 = 0, 0
            name1 = _resolve_arg_fallback(code, op1, idx1)
            name2 = _resolve_arg_fallback(code, op2, idx2)
            result.append([op1, name1])
            result.append([op2, name2])
            continue

        if _is_jump(inst) and isinstance(inst.argval, int):
            target_idx = offset_to_idx.get(inst.argval)
            # Detect unresolved argval (RustPython may not resolve jump targets):
            # 1. argval not in offset_to_idx (not a valid byte offset)
            # 2. argval == arg (raw arg returned as-is, not resolved to offset)
            # 3. For backward jumps: argval should be < current offset
            is_backward = "BACKWARD" in inst.opname
            argval_is_raw = inst.argval == inst.arg and inst.arg is not None
            if target_idx is None or argval_is_raw:
                target_idx = None  # force recalculation
                if is_backward:
                    # Target = current_offset + INSTR_SIZE + cache
                    #        - arg * INSTR_SIZE
                    # Try different cache sizes (NOT_TAKEN=1 for JUMP_BACKWARD, 0 for NO_INTERRUPT)
                    if "NO_INTERRUPT" in inst.opname:
                        cache_order = (0, 1, 2)
                    else:
                        cache_order = (1, 0, 2, 3)
                    for cache in cache_order:
                        target_off = inst.offset + 2 + cache * 2 - inst.arg * 2
                        if target_off >= 0 and target_off in offset_to_idx:
                            target_idx = offset_to_idx[target_off]
                            break
                elif inst.arg is not None:
                    # Forward jumps: compute target offset using cache entry count.
                    # POP_JUMP_IF_* have 1 cache entry (NOT_TAKEN), others have 0.
                    if "POP_JUMP_IF" in inst.opname:
                        cache_order = (1, 0, 2)
                    elif inst.opname == "FOR_ITER":
                        cache_order = (0, 1, 2)
                    elif inst.opname == "SEND":
                        cache_order = (1, 0, 2)
                    else:
                        cache_order = (0, 1, 2)
                    for extra in cache_order:
                        target_off = inst.offset + 2 + extra * 2 + inst.arg * 2
                        if target_off in offset_to_idx:
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
                cmp_str = (
                    _normalize_argrepr(inst.argrepr) if inst.argrepr else str(inst.arg)
                )
            result.append([opname, cmp_str])
        elif inst.arg is not None and inst.argrepr:
            # If argrepr is just a number, try to resolve it via fallback
            # (RustPython may return raw index instead of variable name)
            argrepr = inst.argrepr
            if argrepr.isdigit() or (argrepr.startswith("-") and argrepr[1:].isdigit()):
                resolved = _resolve_arg_fallback(code, opname, inst.arg)
                if isinstance(resolved, str) and not resolved.isdigit():
                    argrepr = resolved
            result.append([opname, _normalize_argrepr(argrepr)])
        elif inst.arg is not None:
            resolved = _resolve_arg_fallback(code, opname, inst.arg)
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
    parser = argparse.ArgumentParser(description="Dump normalized bytecode as JSON")
    parser.add_argument(
        "--base-dir",
        default=None,
        help="Base directory used to compute relative output paths",
    )
    parser.add_argument(
        "--files-from",
        default=None,
        help="Read newline-separated target paths from this file",
    )
    parser.add_argument(
        "targets", nargs="*", help="Python files or directories to process"
    )
    parser.add_argument(
        "--progress",
        type=int,
        default=0,
        help="Print a dot to stderr every N files processed",
    )
    args = parser.parse_args()

    targets = list(args.targets)
    if args.files_from:
        with open(args.files_from, encoding="utf-8") as f:
            targets.extend(line.strip() for line in f if line.strip())

    results = {}
    count = 0
    for target in targets:
        if os.path.isdir(target):
            for root, dirs, files in os.walk(target):
                dirs[:] = sorted(
                    d for d in dirs if d != "__pycache__" and not d.startswith(".")
                )
                for fname in sorted(files):
                    if fname.endswith(".py"):
                        fpath = os.path.join(root, fname)
                        rel_base = args.base_dir or target
                        relpath = os.path.relpath(fpath, rel_base)
                        results[relpath] = process_file(fpath)
                        count += 1
                        if args.progress and count % args.progress == 0:
                            sys.stderr.write(".")
                            sys.stderr.flush()
        elif target.endswith(".py"):
            rel_base = args.base_dir or os.path.dirname(target) or "."
            relpath = os.path.relpath(target, rel_base)
            results[relpath] = process_file(target)
            count += 1
            if args.progress and count % args.progress == 0:
                sys.stderr.write(".")
                sys.stderr.flush()

    json.dump(results, sys.stdout, ensure_ascii=False, separators=(",", ":"))


if __name__ == "__main__":
    main()
