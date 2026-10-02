import ast
import warnings

from testutils import assert_raises, skip_if_unsupported


def test_compile_module():
    class ModuleName(str):
        pass

    for module in (None, "", "package.module", ModuleName("package.module"), "\ud800"):
        code = compile("value = 42", "<module-context>", "exec", module=module)
        assert code.co_filename == "<module-context>"
        namespace = {}
        exec(code, namespace)
        assert namespace["value"] == 42

    for module in (1, False, b"package.module", [], {}, object()):
        with assert_raises(TypeError) as caught:
            compile("", "<module-context>", "exec", module=module)
        assert "argument 'module' must be str or None" in str(caught.exception)

    # module is keyword-only; _feature_version stays keyword-only too.
    with assert_raises(TypeError):
        compile("", "<module-context>", "exec", 0, False, -1, "package.module")

    lexical_sources = [
        "x = 1or 0",
        r"'\z'",
        r"'\400'",
        r'f"{1:\z}"',
        r'f"\{1}"',
    ]
    codegen_sources = [
        "assert (1, 2)",
        "1 is 1",
        "def f():\n    try:\n        pass\n    finally:\n        return 1\n",
    ]

    # Default source compilation uses module for matching, while diagnostics retain
    # the real source filename. Test both text and bytes compilation.
    for source in lexical_sources + codegen_sources:
        for input_source in (source, source.encode()):
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("error", SyntaxWarning)
                warnings.filterwarnings(
                    "always", category=SyntaxWarning, module=r"^package\.module$"
                )
                compile(
                    input_source, "<module-context>", "exec", module="package.module"
                )
            assert len(caught) == 1, (source, caught)
            assert caught[0].filename == "<module-context>"
            assert caught[0].category is SyntaxWarning
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", SyntaxWarning)
                warnings.filterwarnings(
                    "error", category=SyntaxWarning, module=r"^package\.module$"
                )
                with assert_raises(SyntaxError):
                    compile(
                        input_source,
                        "<module-context>",
                        "exec",
                        module="package.module",
                    )

    # Compiling a pre-existing AST must use the module too.
    for source in codegen_sources:
        tree = ast.parse(source)
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("error", SyntaxWarning)
            warnings.filterwarnings(
                "always", category=SyntaxWarning, module=r"^package\.module$"
            )
            compile(tree, "<module-context>", "exec", module="package.module")
        assert len(caught) == 1, (source, caught)
        assert caught[0].filename == "<module-context>"

    # A following compile, with omitted or None module, must not inherit the override.
    for kwargs in ({}, {"module": None}):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("error", SyntaxWarning)
            warnings.filterwarnings(
                "always", category=SyntaxWarning, module=r"^<module-context>$"
            )
            compile("1 is 1", "<module-context>", "exec", **kwargs)
        assert len(caught) == 1

    # Failed parses use the fallback string scanner, which must retain the module.
    for source in (r"b'\z' )", r'f"{1:\z}" )'):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("error", SyntaxWarning)
            warnings.filterwarnings(
                "always", category=SyntaxWarning, module=r"^package\.module$"
            )
            with assert_raises(SyntaxError):
                compile(source, "<module-context>", "exec", module="package.module")
        assert len(caught) == 1, (source, caught)
        assert caught[0].filename == "<module-context>"

    # AST conversion must use the same fallible, source-located warning boundary.
    # TYPE_COMMENTS validates through the AST parser before compiling the source;
    # the later compiler must not filter or emit the same escape a second time.
    for flags in (
        ast.PyCF_ONLY_AST,
        ast.PyCF_TYPE_COMMENTS,
        ast.PyCF_ONLY_AST | ast.PyCF_TYPE_COMMENTS,
        ast.PyCF_ONLY_AST | ast.PyCF_OPTIMIZED_AST,
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

        with warnings.catch_warnings():
            warnings.simplefilter("ignore", SyntaxWarning)
            warnings.filterwarnings(
                "error", category=SyntaxWarning, module=r"^package\.module$"
            )
            with assert_raises(SyntaxError) as caught:
                compile(
                    source, "<module-context>", "exec", flags, module="package.module"
                )
        error = caught.exception
        assert error.filename == "<module-context>"
        assert (error.lineno, error.offset, error.end_lineno, error.end_offset) == (
            2,
            10,
            2,
            12,
        )
        assert error.text == 'π = f"{1:\\z}"\n'
        assert "Such sequences will not work in the future" not in error.msg

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

    # These warnings already came from AST conversion. Retain them instead of
    # replacing that path with a broader scanner before every AST parse.
    for source in ('''rf"{1:{'\\z'}}"''', '''f"{1:{t'\\z'}}"'''):
        for flags in (
            ast.PyCF_ONLY_AST,
            ast.PyCF_TYPE_COMMENTS,
            ast.PyCF_ONLY_AST | ast.PyCF_TYPE_COMMENTS,
        ):
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("error", SyntaxWarning)
                warnings.filterwarnings(
                    "always", category=SyntaxWarning, module=r"^package\.module$"
                )
                compile(
                    source, "<module-context>", "exec", flags, module="package.module"
                )
            assert len(caught) == 1, (source, flags, caught)
            assert (caught[0].filename, caught[0].lineno) == ("<module-context>", 1)

    for flags in (ast.PyCF_ONLY_AST, ast.PyCF_ONLY_AST | ast.PyCF_TYPE_COMMENTS):
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

    # Raw literal text is not an invalid escape. Nested nonraw expressions still
    # retain the diagnostics previously emitted during AST conversion.
    for flags in (
        ast.PyCF_ONLY_AST,
        ast.PyCF_TYPE_COMMENTS,
        ast.PyCF_ONLY_AST | ast.PyCF_TYPE_COMMENTS,
    ):
        for source in (
            r'rf"{1:\z}"',
            r'rf"{1:{2:\z}}"',
            r'tr"{1:{2:\z}}"',
            "f\"{1:{r'\\z'}}\"",
        ):
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("error", SyntaxWarning)
                compile(
                    source, "<module-context>", "exec", flags, module="package.module"
                )
            assert not caught, (source, flags, caught)
        for source in ("rf\"{f'{2:\\z}'}\"", "rf\"{1:{'\\z'}}\""):
            with warnings.catch_warnings():
                warnings.simplefilter("error", SyntaxWarning)
                with assert_raises(SyntaxError):
                    compile(
                        source,
                        "<module-context>",
                        "exec",
                        flags,
                        module="package.module",
                    )

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
