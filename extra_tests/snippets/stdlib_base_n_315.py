"""Focused public-API regressions for native CPython 3.15 base-N codecs."""

import array
import base64
import binascii
import sys
import unittest


class Index:
    def __index__(self):
        return 11


class TrueValue:
    def __bool__(self):
        return True


class BoolError:
    def __bool__(self):
        raise RuntimeError("truth conversion")


class NativeBaseNTests(unittest.TestCase):
    def test_constants(self):
        self.assertEqual(
            binascii.BASE64_ALPHABET,
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
        )
        self.assertEqual(binascii.URLSAFE_BASE64_ALPHABET[-2:], b"-_")
        self.assertEqual(binascii.BASE32_ALPHABET, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567")
        self.assertEqual(
            binascii.BASE32HEX_ALPHABET, b"0123456789ABCDEFGHIJKLMNOPQRSTUV"
        )
        self.assertEqual(binascii.ASCII85_ALPHABET, bytes(range(33, 118)))
        for name, size in [
            ("CRYPT_ALPHABET", 64),
            ("UU_ALPHABET", 64),
            ("BINHEX_ALPHABET", 64),
            ("BASE85_ALPHABET", 85),
            ("Z85_ALPHABET", 85),
        ]:
            value = getattr(binascii, name)
            self.assertIs(type(value), bytes)
            self.assertEqual(len(value), size)
            self.assertEqual(len(set(value)), size)

    def test_base64_padding_canonical_and_strict_default(self):
        self.assertEqual(binascii.b2a_base64(b"a", padded=False), b"YQ\n")
        self.assertEqual(binascii.b2a_base64(b"a", padded=[], newline=[]), b"YQ")
        self.assertEqual(binascii.a2b_base64(b"YQ", padded=False), b"a")
        self.assertEqual(binascii.a2b_base64(b"YQ==@"), b"a")
        with self.assertRaisesRegex(binascii.Error, "Only base64 data"):
            binascii.a2b_base64(b"YQ==@", ignorechars=b"")
        self.assertEqual(binascii.a2b_base64(b"YQ==@", ignorechars=b"@"), b"a")
        self.assertEqual(
            binascii.a2b_base64(b"YQ==@", ignorechars=b"", strict_mode=None), b"a"
        )
        self.assertEqual(
            binascii.a2b_base64(b"Y=Q", padded=False, ignorechars=b"="), b"a"
        )
        with self.assertRaisesRegex(binascii.Error, "Non-zero padding bits"):
            binascii.a2b_base64(b"YR==", canonical=TrueValue())
        with self.assertRaisesRegex(binascii.Error, "Excess data after padding"):
            binascii.a2b_base64(b"YQ==YQ==", strict_mode=True)
        self.assertEqual(binascii.a2b_base64(b"YQ==YQ=="), b"a\x06\x10")

    def test_base32_padding_wrapping_and_canonical(self):
        self.assertEqual(binascii.b2a_base32(b"a"), b"ME======")
        self.assertEqual(binascii.b2a_base32(b"a", padded=False), b"ME")
        self.assertEqual(
            binascii.b2a_base32(b"abcdefghij", wrapcol=11), b"MFRGGZDF\nMZTWQ2LK"
        )
        self.assertEqual(binascii.a2b_base32(b"ME", padded=False), b"a")
        self.assertEqual(binascii.a2b_base32(b"M E=\n=====", ignorechars=b" \n"), b"a")
        with self.assertRaisesRegex(binascii.Error, "Non-zero padding bits"):
            binascii.a2b_base32(b"MF======", canonical=True)
        with self.assertRaisesRegex(binascii.Error, "Leading padding"):
            binascii.a2b_base32(b"=")
        with self.assertRaisesRegex(binascii.Error, "cannot be 1, 3, or 6"):
            binascii.a2b_base32(b"A", padded=False)

    def test_base85_ascii85_and_canonical(self):
        self.assertEqual(binascii.b2a_base85(b"abcd"), b"VPa!s")
        self.assertEqual(binascii.b2a_ascii85(b"abcd"), b"@:E_W")
        self.assertEqual(
            binascii.b2a_ascii85(b"\0" * 4 + b" " * 4, foldspaces=True), b"zy"
        )
        self.assertEqual(
            binascii.b2a_ascii85(b"\0" * 4, adobe=True, wrapcol=1), b"<~\nz\n~>"
        )
        self.assertEqual(
            binascii.a2b_ascii85(b"<~zy~>", adobe=True, foldspaces=True),
            b"\0" * 4 + b" " * 4,
        )
        self.assertEqual(binascii.a2b_ascii85(b"!!!!!"), b"\0" * 4)
        with self.assertRaisesRegex(binascii.Error, "use 'z'"):
            binascii.a2b_ascii85(b"!!!!!", canonical=True)
        with self.assertRaisesRegex(binascii.Error, "Incomplete Base85 group"):
            binascii.a2b_base85(b"0")
        with self.assertRaisesRegex(binascii.Error, "Incomplete Ascii85 group"):
            binascii.a2b_ascii85(b"!")
        with self.assertRaisesRegex(binascii.Error, "Ascii85 overflow"):
            binascii.a2b_ascii85(b"uuuuu")
        with self.assertRaisesRegex(binascii.Error, "Non-zero padding bits"):
            binascii.a2b_base85(b"VG", canonical=True)

    def test_alphabets_and_buffer_types(self):
        groups = [
            (binascii.b2a_base64, binascii.a2b_base64, binascii.CRYPT_ALPHABET),
            (binascii.b2a_base32, binascii.a2b_base32, binascii.BASE32HEX_ALPHABET),
            (binascii.b2a_base85, binascii.a2b_base85, binascii.Z85_ALPHABET),
        ]
        for encode, decode, alphabet in groups:
            expected = encode(b"example", alphabet=alphabet)
            for make in (bytes, bytearray, memoryview, lambda b: array.array("B", b)):
                self.assertEqual(
                    encode(make(b"example"), alphabet=make(alphabet)), expected
                )
                encoded = expected.rstrip(b"\n")
                self.assertEqual(decode(make(encoded), alphabet=alphabet), b"example")
                self.assertEqual(
                    decode(
                        make(encoded + b"\n"),
                        alphabet=alphabet,
                        ignorechars=make(b"\n"),
                    ),
                    b"example",
                )
            for bad in (None, alphabet.decode()):
                with self.assertRaises(TypeError):
                    encode(b"", alphabet=bad)
                with self.assertRaises(TypeError):
                    decode(b"", alphabet=bad)
            with self.assertRaises(TypeError):
                decode(b"", alphabet=bytearray(alphabet))
            for bad in (alphabet[:-1], alphabet + b"@"):
                with self.assertRaises(ValueError):
                    encode(b"", alphabet=bad)
                with self.assertRaises(ValueError):
                    decode(b"", alphabet=bad)
            # Encoders accept duplicate characters; decoders use the last index.
            self.assertEqual(
                len(encode(b"abc", alphabet=b"*" * len(alphabet))), len(encode(b"abc"))
            )

    def test_width_type_and_truth_conversion(self):
        for encode in (
            binascii.b2a_base64,
            binascii.b2a_base32,
            binascii.b2a_base85,
            binascii.b2a_ascii85,
        ):
            self.assertEqual(
                encode(b"abcdefghij", wrapcol=Index()),
                encode(b"abcdefghij", wrapcol=11),
            )
            self.assertEqual(
                encode(b"abc", wrapcol=2 * sys.maxsize + 1), encode(b"abc")
            )
            with self.assertRaises(ValueError):
                encode(b"", wrapcol=-1)
            with self.assertRaises(OverflowError):
                encode(b"", wrapcol=2 * sys.maxsize + 2)
            for bad in (None, 1.0, "1"):
                with self.assertRaises(TypeError):
                    encode(b"", wrapcol=bad)
        for func, option in [
            (binascii.b2a_base64, "padded"),
            (binascii.b2a_base64, "newline"),
            (binascii.a2b_base64, "strict_mode"),
            (binascii.a2b_base32, "canonical"),
            (binascii.b2a_ascii85, "adobe"),
            (binascii.a2b_ascii85, "foldspaces"),
            (binascii.b2a_base85, "pad"),
        ]:
            with self.assertRaisesRegex(RuntimeError, "truth conversion"):
                func(b"", **{option: BoolError()})

    def test_hex_ignorechars_and_error_precedence(self):
        for decode in (binascii.a2b_hex, binascii.unhexlify):
            self.assertEqual(decode(b"4 1\n42", ignorechars=b" \n"), b"AB")
            self.assertEqual(decode("4142", ignorechars=b"4"), b"AB")
            with self.assertRaisesRegex(
                binascii.Error, "Odd number of hexadecimal digits"
            ):
                decode(b"4 \n", ignorechars=b" \n")
            with self.assertRaisesRegex(binascii.Error, "Non-hexadecimal digit found"):
                decode(b"4@")
            with self.assertRaises(TypeError):
                decode(b"", ignorechars=None)

    def test_exact_stdlib_wrappers(self):
        payload = b"hello world\x00\xff"
        for encode, decode in [
            (base64.b64encode, base64.b64decode),
            (base64.b32encode, base64.b32decode),
            (base64.b32hexencode, base64.b32hexdecode),
            (base64.b16encode, base64.b16decode),
            (base64.b85encode, base64.b85decode),
            (base64.a85encode, base64.a85decode),
            (base64.z85encode, base64.z85decode),
        ]:
            self.assertEqual(decode(encode(payload)), payload)
        self.assertEqual(
            base64.urlsafe_b64decode(base64.urlsafe_b64encode(payload, padded=False)),
            payload,
        )
        self.assertEqual(base64.b16decode(b"4 1", ignorechars=b" "), b"A")
        self.assertEqual(
            base64.decodebytes(base64.encodebytes(payload * 20)), payload * 20
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
