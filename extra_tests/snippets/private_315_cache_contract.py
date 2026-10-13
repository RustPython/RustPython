"""Private development ABI reader/writer boundary; no public compatibility claim."""

import _frozen_importlib_external as external
import importlib
import importlib.util
import py_compile
import sys
import tempfile
import zipfile
from pathlib import Path

if sys.implementation.name == "rustpython":
    assert external.MAGIC_NUMBER == bytes((0x16, 0xDA, 13, 10))
    assert sys.implementation.cache_tag == "rustpython-315-devabi2"

    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "cache_subject.py"
        source.write_text("value = 41\n", encoding="ascii")
        code = compile("value = 42\n", "byte_exact/cache_subject.py", "exec")
        stat = source.stat()
        headers = [
            external._code_to_timestamp_pyc(code, int(stat.st_mtime), stat.st_size),
            external._code_to_hash_pyc(
                code, importlib.util.source_hash(source.read_bytes()), True
            ),
            external._code_to_hash_pyc(
                code, importlib.util.source_hash(source.read_bytes()), False
            ),
        ]
        rejected = [3627, 3666, 0xDA15]
        current = 0xDA16

        def load_from(path):
            sys.modules.pop("cache_subject", None)
            sys.path.insert(0, str(path))
            importlib.invalidate_caches()
            try:
                return importlib.import_module("cache_subject").value
            finally:
                sys.path.pop(0)
                sys.modules.pop("cache_subject", None)

        # Source caches must fall back to source on every foreign magic.
        cache = Path(importlib.util.cache_from_source(str(source)))
        cache.parent.mkdir()
        for payload in headers:
            for magic in rejected + [current]:
                candidate = magic.to_bytes(2, "little") + b"\r\n" + payload[4:]
                cache.write_bytes(candidate)
                assert load_from(root) == (42 if magic == current else 41)

        # Sourceless files cannot silently fall back or accept foreign formats.
        source.unlink()
        sourceless = root / "cache_subject.pyc"
        for payload in headers:
            for magic in rejected + [current]:
                sourceless.write_bytes(
                    magic.to_bytes(2, "little") + b"\r\n" + payload[4:]
                )
                if magic == current:
                    assert load_from(root) == 42
                else:
                    try:
                        load_from(root)
                    except ImportError:
                        pass
                    else:
                        raise AssertionError("foreign sourceless cache accepted")

        # ZIP source fallback and pyc-only rejection use the same token.
        for header_index, payload in enumerate(headers):
            for with_source in (False, True):
                for magic in rejected + [current]:
                    archive = root / f"case_{header_index}_{with_source}_{magic}.zip"
                    with zipfile.ZipFile(archive, "w") as output:
                        output.writestr(
                            "cache_subject.pyc",
                            magic.to_bytes(2, "little") + b"\r\n" + payload[4:],
                        )
                        if with_source:
                            output.writestr("cache_subject.py", "value = 41\n")
                    if magic == current or with_source:
                        value = load_from(archive)
                        if magic != current:
                            assert value == 41
                        elif with_source and header_index == 0:
                            # ZIP timestamp validation may select the source.
                            assert value in (41, 42)
                        else:
                            assert value == 42
                    else:
                        try:
                            load_from(archive)
                        except ImportError:
                            pass
                        else:
                            raise AssertionError("foreign ZIP cache accepted")

        # Explicit py_compile writes under -B; all writers must use the private token.
        source.write_text("value = 41\n", encoding="ascii")
        explicit = root / "explicit.pyc"
        previous = sys.dont_write_bytecode
        sys.dont_write_bytecode = True
        try:
            py_compile.compile(str(source), cfile=str(explicit), doraise=True)
            assert explicit.read_bytes()[:4] == external.MAGIC_NUMBER
        finally:
            sys.dont_write_bytecode = previous
