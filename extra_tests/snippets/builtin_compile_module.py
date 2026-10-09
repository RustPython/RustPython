import ast
import warnings

from testutils import assert_raises, skip_if_unsupported


def test_compile_module():
    # AST conversion must use the same fallible, source-located warning boundary.
    # TYPE_COMMENTS validates through the AST parser before compiling the source;
    # the later compiler must not filter or emit the same escape a second time.
    for flags in (
        ast.PyCF_ONLY_AST,
        ast.PyCF_TYPE_COMMENTS,
    ):
        source = '\nπ = f"{1:\\z}"\n'
        for kwargs, expected_module in (
            ({}, "<module-context>"),
            ({"module": None}, "<module-context>"),
            ({"module": "package.module"}, "package.module"),
        ):
            modules = []

            class ModuleMatcher:
                def match(self, module):
                    modules.append(module)
                    return True

            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("always", SyntaxWarning)
                warnings.filters.insert(
                    0, ("always", None, SyntaxWarning, ModuleMatcher(), 0)
                )
                compile(source, "<module-context>", "exec", flags, **kwargs)
            assert modules == [expected_module], (flags, kwargs, modules)
            assert len(caught) == 1, (flags, kwargs, caught)
            assert (caught[0].filename, caught[0].lineno) == ("<module-context>", 2)
        # Warning handlers may raise arbitrary exceptions, which must survive intact.
        failure = RuntimeError("warning handler failed")

        def showwarning(*args, **kwargs):
            raise failure

        with warnings.catch_warnings():
            warnings.simplefilter("always", SyntaxWarning)
            warnings.showwarning = showwarning
            with assert_raises(RuntimeError) as caught:
                compile(
                    source, "<module-context>", "exec", flags, module="package.module"
                )
        assert caught.exception is failure

        # Nested compile calls must not share module overrides or duplicate tracking.
        modules = []
        locations = []

        class ModuleMatcher:
            def match(self, module):
                modules.append(module)
                return True

        def showwarning(message, category, filename, lineno, *args, **kwargs):
            locations.append((filename, lineno))
            if len(locations) == 1:
                compile(r'f"{1:\z}"', "<inner>", "exec", flags, module="package.inner")

        with warnings.catch_warnings():
            warnings.simplefilter("always", SyntaxWarning)
            warnings.filters.insert(
                0, ("always", None, SyntaxWarning, ModuleMatcher(), 0)
            )
            warnings.showwarning = showwarning
            compile(
                'f"{1:\\z}"\nf"{2:\\q}"',
                "<outer>",
                "exec",
                flags,
                module="package.outer",
            )
        assert modules == ["package.outer", "package.inner", "package.outer"], modules
        assert locations == [("<outer>", 1), ("<inner>", 1), ("<outer>", 2)], locations
    for flags in (ast.PyCF_ONLY_AST,):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always", SyntaxWarning)
            with assert_raises(SyntaxError):
                compile(
                    '!!!\nf"{1:\\z}"',
                    "<module-context>",
                    "exec",
                    flags,
                    module="package.module",
                )
        assert not caught

    # A filter changed by showwarning must affect the next distinct diagnostic,
    # rather than treating the already-considered format escape as a new warning.
    with warnings.catch_warnings():
        warnings.simplefilter("always", SyntaxWarning)

        def showwarning(*args, **kwargs):
            warnings.simplefilter("error", SyntaxWarning)

        warnings.showwarning = showwarning
        with assert_raises(SyntaxError) as caught:
            compile(
                'f"{1:\\z}"\n"\\q"',
                "<module-context>",
                "exec",
                ast.PyCF_TYPE_COMMENTS,
                module="package.module",
            )
    assert caught.exception.lineno == 2
    assert "\\q" in caught.exception.msg
    # Func-type parsing trims its source before converting the parsed nodes.
    # Already-detected escapes must still point into the original source.
    for prefix in ("", "\n\n"):
        source = prefix + '(int) -> f"{1:aaaaaaaaaaaa\\zbbbbbbbbbbbb}"'
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always", SyntaxWarning)
            compile(
                source,
                "<module-context>",
                "func_type",
                ast.PyCF_ONLY_AST,
                module="package.module",
            )
        assert len(caught) == 1, (source, caught)
        assert caught[0].filename == "<module-context>"
        assert caught[0].lineno == prefix.count("\n") + 1

    # Warn for the first invalid escape in each original literal, including
    # literals later folded together. Comments are not literal source text.
    for source, count in (
        (r'f"{1:\z\q}"', 1),
        (r'f"{1:\{2}}"', 1),
        (r'f"{1:\\{2}}"', 0),
        ('f"{1:{(\n1 # \\z\n)}}"', 0),
        ("f\"{1:{'\\z' + '\\q'}}\"", 2),
        ("f\"{1:{'\\z' '\\q'}}\"", 2),
        ("f\"{1:{('\\z', '\\q')[0]}}\"", 2),
    ):
        for flags in (
            ast.PyCF_ONLY_AST,
            ast.PyCF_TYPE_COMMENTS,
            ast.PyCF_ONLY_AST | ast.PyCF_OPTIMIZED_AST,
        ):
            for optimize in (0, 2):
                with warnings.catch_warnings(record=True) as caught:
                    warnings.simplefilter("always", SyntaxWarning)
                    compile(
                        source,
                        "<module-context>",
                        "exec",
                        flags,
                        optimize=optimize,
                        module="package.module",
                    )
                assert len(caught) == count, (source, flags, optimize, caught)
                assert all(w.filename == "<module-context>" for w in caught)


skip_if_unsupported(3, 15, test_compile_module)
