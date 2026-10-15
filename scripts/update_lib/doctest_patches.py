"""Apply explicitly reviewed exact-source doctest metadata to canonical input."""

import ast
import doctest
import json
import re
from pathlib import Path

EXPECTED_FAILURE = doctest.register_optionflag("EXPECTED_FAILURE")
CANONICAL_LOAD_TESTS = """def load_tests(loader, tests, pattern):
    tests.addTest(doctest.DocTestSuite())
    return tests"""
PATCHED_LOAD_TESTS = """def load_tests(loader, tests, pattern):
    from test.support.rustpython import DocTestChecker  # TODO: RUSTPYTHON
    tests.addTest(doctest.DocTestSuite(checker=DocTestChecker()))  # TODO: RUSTPYTHON
    return tests"""


def _examples(contents):
    tree = ast.parse(contents)
    if not tree.body or not isinstance(tree.body[0], ast.Expr):
        raise ValueError("Missing module doctest")
    doc = tree.body[0].value
    if not isinstance(doc, ast.Constant) or not isinstance(doc.value, str):
        raise ValueError("Missing module doctest")
    # Raw docstring offsets map directly to source lines, unlike cleaned docs.
    return tree, doc, doctest.DocTestParser().get_examples(doc.value)


def apply_doctest_metadata(old, canonical, metadata):
    """Preserve only listed guards, without carrying old expected output."""
    _, _, old_examples = _examples(old)
    tree, doc, examples = _examples(canonical)
    lines = canonical.splitlines(keepends=True)
    for source in metadata:
        old_matches = [
            example
            for example in old_examples
            if re.sub(r"\s+# TODO: RUSTPYTHON[^\n]*", "", example.source) == source
        ]
        if len(old_matches) != 1 or not old_matches[0].options.get(EXPECTED_FAILURE):
            raise ValueError("Missing or ambiguous existing doctest guard")
        matches = [example for example in examples if example.source == source]
        if len(matches) != 1 or matches[0].options:
            raise ValueError("Missing, ambiguous, or already marked canonical doctest")
        index = doc.lineno - 1 + matches[0].lineno
        line = lines[index]
        if (
            ">>>" not in line
            or line.split(">>>", 1)[1].strip() != source.splitlines()[0]
        ):
            raise ValueError("Unrecognized canonical doctest prompt")
        lines[index] = (
            line.rstrip("\n")
            + "  # TODO: RUSTPYTHON; "
            + metadata[source]
            + " # doctest: +EXPECTED_FAILURE\n"
        )

    loaders = [
        node
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name == "load_tests"
    ]
    if len(loaders) != 1 or loaders[0] not in tree.body:
        raise ValueError("Missing or ambiguous doctest loader")
    if ast.get_source_segment(canonical, loaders[0]) != CANONICAL_LOAD_TESTS:
        raise ValueError("Changed canonical doctest loader")
    result = "".join(lines)
    if result.count(CANONICAL_LOAD_TESTS) != 1:
        raise ValueError("Ambiguous canonical doctest loader text")
    return result.replace(CANONICAL_LOAD_TESTS, PATCHED_LOAD_TESTS, 1)


def preserve_doctest_patch(relative_path: Path, old: str, canonical: str) -> str:
    if relative_path.parts != ("Lib", "test", "test_syntax.py"):
        return canonical
    metadata = json.loads(
        (Path(__file__).parent / "patches" / "test_syntax.json").read_text(
            encoding="utf-8"
        )
    )
    return apply_doctest_metadata(old, canonical, metadata)
