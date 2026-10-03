"""Registry key/warning/cache regressions against CPython 3.15 rc2."""

import codecs
import encodings
import unittest
import warnings

WARNING = "Support for non-ascii encoding names will be removed in 3.17"


def clear_native_cache():
    def sentinel(name):
        return None

    codecs.register(sentinel)
    codecs.unregister(sentinel)


class RegistryNormalizationTests(unittest.TestCase):
    def setUp(self):
        clear_native_cache()
        encodings._cache.clear()

    def tearDown(self):
        clear_native_cache()
        encodings._cache.clear()

    def test_search_function_receives_registry_normalization(self):
        seen = []

        def search(name):
            if name.startswith("codecprobe."):
                seen.append(name)
                return name, 2, 3, 4

        codecs.register(search)
        self.addCleanup(codecs.unregister, search)
        pairs = [
            ("CODECPROBE.AAA 8", "codecprobe.aaa-8"),
            ("CODECPROBE.AAA---8", "codecprobe.aaa---8"),
            ("CODECPROBE.AAA   8", "codecprobe.aaa---8"),
            ("CODECPROBE.AAA_8", "codecprobe.aaa_8"),
            ("CODECPROBE.AAA...8", "codecprobe.aaa...8"),
            ("CODECPROBE.Éé\t/A", "codecprobe.Éé\t/a"),
        ]
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            for original, expected in pairs:
                self.assertEqual(codecs.lookup(original)[0], expected)
        self.assertEqual(
            seen,
            [
                "codecprobe.aaa-8",
                "codecprobe.aaa---8",
                "codecprobe.aaa_8",
                "codecprobe.aaa...8",
                "codecprobe.Éé\t/a",
            ],
        )
        self.assertEqual([str(w.message) for w in caught], [WARNING])

    def test_positive_aliases_keep_separate_warning_cache_keys(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            for name in ("utf_8é", "UTF_8é", "utf_8é", "utf_8€", "utf_8€"):
                self.assertEqual(codecs.lookup(name).name, "utf-8")
        self.assertEqual([str(w.message) for w in caught], [WARNING, WARNING])
        self.assertIn("utf_8é", encodings._cache)
        self.assertIn("utf_8€", encodings._cache)

    def test_negative_lookup_warning_is_cached_in_encodings(self):
        original = "CodecMissingÉ--A"
        key = "codecmissingÉ--a"
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            for _ in range(3):
                with self.assertRaises(LookupError) as cm:
                    codecs.lookup(original)
                self.assertEqual(str(cm.exception), f"unknown encoding: {original}")
            self.assertIn(key, encodings._cache)
            self.assertIsNone(encodings._cache[key])
        self.assertEqual([str(w.message) for w in caught], [WARNING])
        encodings._cache.clear()
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            with self.assertRaises(LookupError):
                codecs.lookup(original)
        self.assertEqual([str(w.message) for w in caught], [WARNING])

    def test_warning_error_does_not_cache_a_failed_search(self):
        name = "CodecMissingÉ--B"
        with warnings.catch_warnings():
            warnings.simplefilter("error", DeprecationWarning)
            for _ in range(2):
                with self.assertRaises(DeprecationWarning):
                    codecs.lookup(name)
        self.assertNotIn("codecmissingÉ--b", encodings._cache)

    def test_negative_native_lookup_does_not_block_later_search_function(self):
        name = "CodecMissingé--C"
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            with self.assertRaises(LookupError):
                codecs.lookup(name)
            called = []

            def search(key):
                if key == "codecmissingé--c":
                    called.append(key)
                    return codecs.lookup("ascii")

            codecs.register(search)
            try:
                self.assertEqual(codecs.lookup(name).name, "ascii")
                self.assertEqual(codecs.lookup(name).name, "ascii")
            finally:
                codecs.unregister(search)
        self.assertEqual(called, ["codecmissingé--c"])
        self.assertEqual([str(w.message) for w in caught], [WARNING])

    def test_python_cache_clear_does_not_invalidate_native_success(self):
        name = "utf_8é"
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            codecs.lookup(name)
            encodings._cache.clear()
            codecs.lookup(name)
        self.assertEqual([str(w.message) for w in caught], [WARNING])
        clear_native_cache()
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            codecs.lookup(name)
        self.assertEqual([str(w.message) for w in caught], [WARNING])

    def test_builtin_fast_path_still_bypasses_registry_warning(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            self.assertEqual(b"example".decode("utf-8é"), "example")
            self.assertEqual("example".encode("utf-8é"), b"example")
        self.assertEqual(caught, [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
