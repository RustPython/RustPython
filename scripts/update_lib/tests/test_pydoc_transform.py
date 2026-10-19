"""Host-tooling tests for the explicitly approved pydoc layout adaptation."""

import ast
import contextlib
import hashlib
import inspect
import io
import os
import pathlib
import sys
import tempfile
import types
import unittest
import warnings
from unittest.mock import patch

from update_lib.cmd_copy_lib import copy_lib, main
from update_lib.lib_transforms import (
    PYDOC_SOURCE_SHA256,
    PYDOC_STDLIB_ANCHOR,
    PYDOC_STDLIB_REPLACEMENT,
    pydoc_build_stdlib,
)


def canonical_pydoc():
    # Recover the pinned input from the checked-in generated file, so these
    # tests need neither an external CPython checkout nor a second stdlib copy.
    path = pathlib.Path(__file__).resolve().parents[3] / "Lib/pydoc.py"
    contents = path.read_bytes()
    if contents.count(PYDOC_STDLIB_REPLACEMENT) == 1:
        contents = contents.replace(PYDOC_STDLIB_REPLACEMENT, PYDOC_STDLIB_ANCHOR, 1)
    if hashlib.sha256(contents).hexdigest() != PYDOC_SOURCE_SHA256:
        raise AssertionError("Checked-in pydoc no longer matches the reviewed input")
    return contents


class TestPydocTransform(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = canonical_pydoc()

    def test_changes_only_default_stdlib_assignment(self):
        original = b"    STDLIB_DIR = sysconfig.get_path('stdlib')\n"
        expected = (
            original
            + b"""    # TODO: RUSTPYTHON; Cargo builds keep Lib outside sysconfig's install paths.
    if (sys.implementation.name == 'rustpython'
            and getattr(sys, '_is_python_build', False)
            and not sysconfig.is_python_build()
            and getattr(sys, '_stdlib_dir', None)):
        STDLIB_DIR = sys._stdlib_dir
"""
        )
        before, found, after = self.source.partition(original)
        self.assertEqual(found, original)
        self.assertEqual(pydoc_build_stdlib(self.source), before + expected + after)

    def test_checked_in_pydoc_matches_generated_bytes(self):
        path = pathlib.Path(__file__).resolve().parents[3] / "Lib/pydoc.py"
        self.assertEqual(path.read_bytes(), pydoc_build_stdlib(self.source))

    def test_refuses_changed_source(self):
        with self.assertRaisesRegex(ValueError, "SHA256"):
            pydoc_build_stdlib(self.source + b"# unreviewed change\n")

    def test_requires_exactly_one_anchor(self):
        for contents in (
            self.source.replace(PYDOC_STDLIB_ANCHOR, b""),
            self.source + PYDOC_STDLIB_ANCHOR,
            pydoc_build_stdlib(self.source),
        ):
            with self.subTest(contents_length=len(contents)):
                with self.assertRaisesRegex(ValueError, "one canonical.*anchor"):
                    pydoc_build_stdlib(contents)

    def test_explicit_copy_is_repeatable_and_changes_only_pydoc(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            source = root / "cpython/Lib/pydoc.py"
            source.parent.mkdir(parents=True)
            source.write_bytes(self.source)
            dependency = source.parent / "pydoc_data/module_docs.py"
            dependency.parent.mkdir()
            dependency.write_bytes(b"unselected dependency")
            target = root / "Lib/pydoc.py"
            target.parent.mkdir()
            target.write_bytes(b"old pydoc")
            sibling = target.parent / "pydoc_data/module_docs.py"
            sibling.parent.mkdir()
            sibling.write_bytes(b"unchanged dependency")
            with contextlib.chdir(root), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main([str(source), "--pydoc-build-stdlib"]), 0)
                first = target.read_bytes()
                copy_lib(source, verbose=False, pydoc_build_stdlib=True)
            self.assertEqual(first, pydoc_build_stdlib(self.source))
            self.assertEqual(target.read_bytes(), first)
            self.assertEqual(source.read_bytes(), self.source)
            self.assertEqual(sibling.read_bytes(), b"unchanged dependency")

    def test_ordinary_copy_keeps_canonical_bytes_and_dependencies(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            source = root / "cpython/Lib/pydoc.py"
            source.parent.mkdir(parents=True)
            source.write_bytes(self.source)
            dependency = source.parent / "pydoc_data/module_docs.py"
            dependency.parent.mkdir()
            dependency.write_bytes(b"canonical dependency")
            with contextlib.chdir(root):
                copy_lib(source, verbose=False)
            self.assertEqual((root / "Lib/pydoc.py").read_bytes(), self.source)
            self.assertEqual(
                (root / "Lib/pydoc_data/module_docs.py").read_bytes(),
                b"canonical dependency",
            )

    def test_refuses_unrelated_paths_without_writing(self):
        for relative in ("other.py", "test/pydoc.py", "package/../pydoc.py"):
            with self.subTest(path=relative), tempfile.TemporaryDirectory() as tmpdir:
                root = pathlib.Path(tmpdir)
                source = root / "cpython/Lib" / relative
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_bytes(self.source)
                with contextlib.chdir(root):
                    with self.assertRaisesRegex(ValueError, "exactly Lib/pydoc.py"):
                        copy_lib(source, verbose=False, pydoc_build_stdlib=True)
                self.assertFalse((root / "Lib").exists())

    def test_refuses_directory_source_without_writing(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            source = root / "cpython/Lib/pydoc.py"
            source.mkdir(parents=True)
            with contextlib.chdir(root):
                with self.assertRaisesRegex(ValueError, "exactly Lib/pydoc.py"):
                    copy_lib(source, verbose=False, pydoc_build_stdlib=True)
            self.assertFalse((root / "Lib").exists())

    def test_invalid_input_preserves_existing_destination(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            source = root / "cpython/Lib/pydoc.py"
            source.parent.mkdir(parents=True)
            source.write_bytes(self.source + b"# unreviewed change\n")
            target = root / "Lib/pydoc.py"
            target.parent.mkdir()
            target.write_bytes(b"preserve destination")
            with contextlib.chdir(root), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main([str(source), "--pydoc-build-stdlib"]), 1)
            self.assertEqual(target.read_bytes(), b"preserve destination")

    def test_refuses_source_as_destination(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            source = root / "Lib/pydoc.py"
            source.parent.mkdir()
            source.write_bytes(self.source)
            with contextlib.chdir(root):
                with self.assertRaisesRegex(ValueError, "different files"):
                    copy_lib(source, verbose=False, pydoc_build_stdlib=True)
            self.assertEqual(source.read_bytes(), self.source)

    def test_refuses_nonregular_destination(self):
        for kind in ("directory", "symlink"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmpdir:
                root = pathlib.Path(tmpdir)
                source = root / "cpython/Lib/pydoc.py"
                source.parent.mkdir(parents=True)
                source.write_bytes(self.source)
                target = root / "Lib/pydoc.py"
                target.parent.mkdir()
                other = root / "other.py"
                other.write_bytes(b"preserve other file")
                if kind == "directory":
                    target.mkdir()
                else:
                    target.symlink_to(other)
                with contextlib.chdir(root):
                    with self.assertRaisesRegex(ValueError, "regular file"):
                        copy_lib(source, verbose=False, pydoc_build_stdlib=True)
                self.assertEqual(other.read_bytes(), b"preserve other file")
                self.assertEqual(source.read_bytes(), self.source)

    def test_refuses_hardlinked_source_and_destination(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            source = root / "cpython/Lib/pydoc.py"
            source.parent.mkdir(parents=True)
            source.write_bytes(self.source)
            target = root / "Lib/pydoc.py"
            target.parent.mkdir()
            target.hardlink_to(source)
            with contextlib.chdir(root):
                with self.assertRaisesRegex(ValueError, "different files"):
                    copy_lib(source, verbose=False, pydoc_build_stdlib=True)
            self.assertEqual(source.read_bytes(), self.source)


class TestPydocPathSelection(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = canonical_pydoc()
        cls.generated = pydoc_build_stdlib(cls.source)

    def make_doc(
        self,
        *,
        source=None,
        implementation="rustpython",
        config_build=False,
        runtime_attributes=None,
    ):
        source = self.generated if source is None else source
        tree = ast.parse(source)
        doc = next(
            node
            for node in tree.body
            if isinstance(node, ast.ClassDef) and node.name == "Doc"
        )
        runtime = types.SimpleNamespace(
            implementation=types.SimpleNamespace(name=implementation),
            version_info=(3, 15),
            **(runtime_attributes or {}),
        )
        namespace = {
            "os": os,
            "sys": runtime,
            "sysconfig": types.SimpleNamespace(
                get_path=lambda name: os.path.abspath("installed/stdlib"),
                is_python_build=lambda: config_build,
                get_config_var=lambda name: os.path.abspath("source"),
            ),
            "inspect": inspect,
            "warnings": warnings,
        }
        exec(
            compile(ast.Module(body=[doc], type_ignores=[]), "pydoc.py", "exec"),
            namespace,
        )
        return namespace["Doc"]()

    def test_only_cargo_build_mismatch_changes_default(self):
        runtime_dir = os.path.abspath("checkout/Lib")
        for implementation in ("rustpython", "cpython", "pypy"):
            for config_build in (False, True):
                for attributes in (
                    {},
                    {"_stdlib_dir": runtime_dir},
                    {"_is_python_build": False, "_stdlib_dir": runtime_dir},
                    {"_is_python_build": True},
                    {"_is_python_build": True, "_stdlib_dir": None},
                    {"_is_python_build": True, "_stdlib_dir": ""},
                    {"_is_python_build": True, "_stdlib_dir": runtime_dir},
                ):
                    with self.subTest(
                        implementation=implementation,
                        config_build=config_build,
                        attributes=attributes,
                    ):
                        options = dict(
                            implementation=implementation,
                            config_build=config_build,
                            runtime_attributes=attributes,
                        )
                        doc = self.make_doc(**options)
                        original = self.make_doc(source=self.source, **options)
                        adapted = (
                            implementation == "rustpython"
                            and not config_build
                            and attributes.get("_is_python_build")
                            and attributes.get("_stdlib_dir")
                        )
                        self.assertEqual(
                            doc.STDLIB_DIR,
                            runtime_dir if adapted else original.STDLIB_DIR,
                        )
                        module = types.ModuleType("example")
                        for directory in (
                            runtime_dir,
                            original.STDLIB_DIR,
                            os.path.abspath("source/Lib"),
                        ):
                            module.__file__ = os.path.join(directory, "example.py")
                            if not adapted:
                                self.assertEqual(
                                    doc._is_stdlib_module(module),
                                    original._is_stdlib_module(module),
                                )
                            for explicit in (runtime_dir, original.STDLIB_DIR, ""):
                                self.assertEqual(
                                    doc._is_stdlib_module(module, explicit),
                                    original._is_stdlib_module(module, explicit),
                                )

    def test_runtime_stdlib_detection_and_site_packages_exclusion(self):
        runtime_dir = os.path.abspath("checkout/Lib")
        doc = self.make_doc(
            runtime_attributes={"_is_python_build": True, "_stdlib_dir": runtime_dir}
        )
        module = types.ModuleType("example")
        module.__file__ = os.path.join(runtime_dir, "example.py")
        self.assertTrue(doc._is_stdlib_module(module))
        self.assertFalse(doc._is_stdlib_module(module, os.path.abspath("custom")))
        module.__file__ = os.path.join(runtime_dir, "site-packages/example.py")
        self.assertFalse(doc._is_stdlib_module(module))

    def test_getdocloc_uses_runtime_default_and_preserves_explicit_basedir(self):
        runtime_dir = os.path.abspath("checkout/Lib")
        doc = self.make_doc(
            runtime_attributes={"_is_python_build": True, "_stdlib_dir": runtime_dir}
        )
        module = types.ModuleType("example")
        module.__file__ = os.path.join(runtime_dir, "example.py")
        data = types.ModuleType("pydoc_data")
        data.module_docs = types.SimpleNamespace(
            module_docs={"example": "example.html"}
        )
        with (
            patch.dict(sys.modules, {"pydoc_data": data}),
            patch.dict(os.environ, {"PYTHONDOCS": "https://docs.example/library"}),
        ):
            self.assertEqual(
                doc.getdocloc(module), "https://docs.example/library/example.html"
            )
            self.assertIsNone(doc.getdocloc(module, os.path.abspath("custom")))

    def test_version_warning_suppression_uses_runtime_stdlib(self):
        class DeprecatedModule(types.ModuleType):
            def __getattr__(self, name):
                if name == "__version__":
                    warnings.warn("deprecated version", DeprecationWarning)
                    return "1.0"
                raise AttributeError(name)

        runtime_dir = os.path.abspath("checkout/Lib")
        doc = self.make_doc(
            runtime_attributes={"_is_python_build": True, "_stdlib_dir": runtime_dir}
        )
        module = DeprecatedModule("example")
        for relative, count in (("example.py", 0), ("site-packages/example.py", 1)):
            with (
                self.subTest(path=relative),
                warnings.catch_warnings(record=True) as caught,
            ):
                warnings.simplefilter("always")
                module.__file__ = os.path.join(runtime_dir, relative)
                self.assertEqual(doc._get_version(module), "1.0")
                self.assertEqual(len(caught), count)


if __name__ == "__main__":
    unittest.main()
