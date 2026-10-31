"""Source-only tests: adapted modules are never imported or executed."""

import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from update_lib.cmd_migrate import migrate_new_file


class TestImportBindingRewrites(unittest.TestCase):
    optional_source = "import native\nfrom test.support import import_helper\n"
    optional_result = (
        "from test.support import import_helper\n"
        "native = import_helper.import_module('native')\n"
    )
    optional = {"canonical_import": "import native", "kind": "optional_module"}
    prefix = (
        "from test import support\n"
        "from test.support import import_helper\n"
        "native = import_helper.import_module('native')\n"
    )
    source = prefix + "from native import first, second as alias\n"
    result = prefix + (
        "first = support.get_attribute(native, 'first')\n"
        "alias = support.get_attribute(native, 'second')\n"
    )
    attributes = {
        "canonical_import": "from native import first, second as alias",
        "kind": "required_attributes",
        "module_binding": "native",
    }
    body = "class Example:\n    def test_value(self):\n        assert first() == 1\n"

    def migrate(self, source, result, declarations, accepted=True, **changes):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            canonical = root / "cpython/Lib/test/test_example.py"
            canonical.parent.mkdir(parents=True)
            canonical.write_text(source)
            repository = root / "rustpython"
            target = repository / "Lib/test/test_example.py"
            document = {
                "target": "Lib/test/test_example.py",
                "source_sha256": hashlib.sha256(canonical.read_bytes()).hexdigest(),
                "blocks": [[source, result]],
                "import_binding_rewrites": declarations,
            }
            document.update(changes)
            with patch(
                "update_lib.cmd_migrate.__file__",
                str(repository / "scripts/update_lib/cmd_migrate.py"),
            ):
                if accepted:
                    migrate_new_file(canonical, document)
                    self.assertEqual(target.read_text(), result)
                else:
                    with self.assertRaises(ValueError):
                        migrate_new_file(canonical, document)
                    self.assertFalse(target.exists())
            self.assertEqual(canonical.read_text(), source)

    def test_optional_and_attribute_rewrites(self):
        self.migrate(self.optional_source, self.optional_result, [self.optional])
        self.migrate(
            self.source + self.body, self.result + self.body, [self.attributes]
        )
        self.migrate(
            self.optional_source.replace("import native", "import native as api"),
            self.optional_result.replace("native =", "api ="),
            [{"canonical_import": "import native as api", "kind": "optional_module"}],
        )

    def test_explicit_opt_in_and_unused_or_invalid_declarations(self):
        for declarations in (
            None,
            [],
            {},
            [self.optional],
            [self.attributes, self.attributes],
            [{**self.attributes, "extra": True}],
            [{**self.attributes, "kind": "anything"}],
            [{**self.attributes, "module_binding": "native.other"}],
        ):
            with self.subTest(declarations=declarations):
                self.migrate(self.source, self.result, declarations, False)
        self.migrate(self.source, self.source, [self.attributes], False)

    def test_wrong_calls_names_order_and_extra_bindings(self):
        replacements = [
            self.result.replace("first =", "renamed ="),
            self.result.replace("'first'", "'wrong'"),
            self.result.replace("get_attribute(native,", "get_attribute(other,"),
            self.result.replace("support.get_attribute", "getattr"),
            self.result.replace(
                "get_attribute(native, 'first')", "get_attribute(native, name='first')"
            ),
            self.result.replace(
                "get_attribute(native, 'first')", "get_attribute(*(native, 'first'))"
            ),
            self.result.replace(
                "get_attribute(native, 'first')",
                "get_attribute(native, 'first') or None",
            ),
            self.result.replace("first =", "first = other ="),
            self.result.replace("first =", "first: object ="),
            self.result.replace(
                "first = support.get_attribute(native, 'first')\nalias = support.get_attribute(native, 'second')",
                "alias = support.get_attribute(native, 'second')\nfirst = support.get_attribute(native, 'first')",
            ),
            self.result + "other = 1\n",
            self.result + "from native import first, second as alias\n",
            self.result + "first = support.get_attribute(native, 'first')\n",
            self.result.replace(
                "from test import support", "from other import support"
            ),
            self.result.replace("import_module('native')", "import_module('other')"),
        ]
        for result in replacements:
            with self.subTest(result=result):
                self.migrate(self.source, result, [self.attributes], False)
        self.migrate(
            self.optional_source,
            self.optional_result.replace("'native'", "'other'"),
            [self.optional],
            False,
        )

    def test_scope_aware_shadow_rejection(self):
        for shadow in (
            "support = None\n",
            "del support\n",
            "native += 1\n",
            "alias: int = 0\n",
            "if True:\n    first = 1\n",
            "for support in []:\n    pass\n",
            "with context() as native:\n    pass\n",
            "try:\n    pass\nexcept Exception as first:\n    pass\n",
            "(support := None)\n",
            "support.get_attribute = other\n",
            "import other as native\n",
            "from other import *\n",
            "def first():\n    pass\n",
            "match value:\n    case support:\n        pass\n",
            "match value:\n    case [*native]:\n        pass\n",
            "match value:\n    case {**alias}:\n        pass\n",
            "class Nested:\n    support.get_attribute = other\n",
        ):
            with self.subTest(shadow=shadow):
                self.migrate(
                    self.source + shadow, self.result + shadow, [self.attributes], False
                )
        local = "def independent():\n    support = None\n    native = None\n"
        self.migrate(self.source + local, self.result + local, [self.attributes])

    def test_order_body_decorator_and_original_binding_stay_protected(self):
        for result in (
            self.result + self.body.replace("== 1", "== 2"),
            self.result + "@skip()\n" + self.body,
            self.result.replace("native =", "other =") + self.body,
            self.prefix + "flag = 1\n" + self.result[len(self.prefix) :] + self.body,
        ):
            self.migrate(self.source + self.body, result, [self.attributes], False)
        self.migrate(
            self.optional_source + "flag = 1\n",
            "flag = 1\n" + self.optional_result,
            [self.optional],
            False,
        )
        self.migrate(
            "import native\nflag = 1\nfrom test.support import import_helper\n",
            self.optional_result + "flag = 1\n",
            [self.optional],
            False,
        )
        self.migrate(
            self.source,
            self.result.replace("from test import support\n", "")
            + "from test import support\n",
            [self.attributes],
            False,
        )

    def test_unsupported_import_forms_and_duplicate_aliases(self):
        for statement, kind in (
            ("import native.sub", "optional_module"),
            ("import native.sub as api", "optional_module"),
            ("import native, other", "optional_module"),
            ("from .native import first", "required_attributes"),
            ("from native.sub import first", "required_attributes"),
            ("from native import *", "required_attributes"),
            ("from native import first, second as first", "required_attributes"),
        ):
            declaration = {"canonical_import": statement, "kind": kind}
            if kind == "required_attributes":
                declaration["module_binding"] = "native"
            self.migrate(
                self.prefix + statement + "\n", self.result, [declaration], False
            )

    def test_helpers_and_module_cannot_be_rewrite_targets(self):
        for name in ("support", "import_helper", "native"):
            source = self.prefix + f"from native import first as {name}\n"
            result = self.prefix + f"{name} = support.get_attribute(native, 'first')\n"
            declaration = {
                **self.attributes,
                "canonical_import": f"from native import first as {name}",
            }
            self.migrate(source, result, [declaration], False)
        self.migrate(
            "import native as import_helper\nfrom test.support import import_helper\n",
            "from test.support import import_helper\nimport_helper = import_helper.import_module('native')\n",
            [
                {
                    "canonical_import": "import native as import_helper",
                    "kind": "optional_module",
                }
            ],
            False,
        )

    def test_existing_hash_and_overlap_checks_remain(self):
        self.migrate(
            self.source, self.result, [self.attributes], False, source_sha256="0" * 64
        )
        self.migrate(
            self.source,
            self.result,
            [self.attributes],
            False,
            blocks=[[self.source, self.result], ["from native", "from other"]],
        )
