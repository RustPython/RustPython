"""Pinned CPython reference probes; does not execute or modify Rust sources."""

import binascii
import inspect
import sys


class Exporter:
    def __init__(self, payload, on_release=None):
        self.payload = payload
        self.on_release = on_release
        self.events = []

    def __buffer__(self, flags):
        self.events.append(("acquire", flags))
        return memoryview(self.payload)

    def __release_buffer__(self, view):
        self.events.append(("release", bytes(view)))
        if self.on_release:
            self.on_release()


def main():
    print(sys.version)
    for name, alphabet in [
        ("b2a_base64", binascii.BASE64_ALPHABET),
        ("b2a_base32", binascii.BASE32_ALPHABET),
        ("b2a_base85", binascii.BASE85_ALPHABET),
    ]:
        payload = bytearray(b"a")
        exporter = Exporter(alphabet, lambda: payload.__setitem__(0, ord("z")))
        fn = getattr(binascii, name)
        result = fn(payload, alphabet=exporter)
        assert result == fn(b"a")
        assert payload == b"z"
        assert [e[0] for e in exporter.events] == ["acquire", "release"]
        print(
            "release-before-read hazard",
            name,
            "reference output",
            repr(result),
            "input after release",
            repr(bytes(payload)),
            "output if prematurely released",
            repr(fn(payload)),
        )

    for name, data, changed in [
        ("a2b_base64", b"YQ==", b"eg=="),
        ("a2b_base32", b"ME======", b"PI======"),
        ("a2b_base85", b"VE", b"dH"),
        ("a2b_ascii85", b"@/", b"H2"),
        ("unhexlify", b"61", b"7a"),
    ]:
        payload = bytearray(data)
        exporter = Exporter(b"", lambda: payload.__setitem__(slice(None), changed))
        fn = getattr(binascii, name)
        result = fn(payload, ignorechars=exporter)
        assert result == fn(data)
        assert bytes(payload) == changed
        print(
            "release-before-read hazard",
            name,
            "reference output",
            repr(result),
            "input after release",
            repr(bytes(payload)),
            "output if prematurely released",
            repr(fn(payload)),
        )

    for name, data, keyword, auxiliary in [
        ("b2a_base64", b"a", "alphabet", binascii.BASE64_ALPHABET),
        ("b2a_base32", b"a", "alphabet", binascii.BASE32_ALPHABET),
        ("b2a_base85", b"a", "alphabet", binascii.BASE85_ALPHABET),
        ("a2b_base64", b"YQ==", "ignorechars", b""),
        ("a2b_base32", b"ME======", "ignorechars", b""),
        ("a2b_base85", b"VE", "ignorechars", b""),
        ("a2b_ascii85", b"@/", "ignorechars", b""),
        ("a2b_hex", b"61", "ignorechars", b""),
        ("unhexlify", b"61", "ignorechars", b""),
    ]:
        releases = []
        payload = bytearray(data)

        def on_auxiliary_release():
            releases.append(keyword)
            payload.extend(b"!")

        primary = Exporter(payload, lambda: releases.append("data"))
        auxiliary = Exporter(auxiliary, on_auxiliary_release)
        fn = getattr(binascii, name)
        result = fn(primary, **{keyword: auxiliary})
        assert result == fn(data)
        assert releases == ["data", keyword]
        assert payload == data + b"!"
        print("release-order", name, releases, "resize succeeds", repr(payload))

    for name, data, keyword, auxiliary in [
        ("b2a_base64", b"a", "alphabet", b"invalid"),
        ("b2a_base32", b"a", "alphabet", b"invalid"),
        ("b2a_base85", b"a", "alphabet", b"invalid"),
        ("a2b_base64", b"A", "ignorechars", b""),
        ("a2b_base32", b"A", "ignorechars", b""),
        ("a2b_base85", b"0", "ignorechars", b""),
        ("a2b_ascii85", b"!", "ignorechars", b""),
        ("a2b_hex", b"A", "ignorechars", b""),
        ("unhexlify", b"A", "ignorechars", b""),
    ]:
        releases = []
        payload = bytearray(data)

        def on_auxiliary_release():
            releases.append(keyword)
            payload.extend(b"!")

        primary = Exporter(payload, lambda: releases.append("data"))
        auxiliary = Exporter(auxiliary, on_auxiliary_release)
        fn = getattr(binascii, name)
        try:
            fn(primary, **{keyword: auxiliary})
        except ValueError as exc:
            print("error-release-order", name, releases, type(exc).__name__, str(exc))
        else:
            raise AssertionError("codec unexpectedly succeeded")
        assert releases == ["data", keyword]
        assert payload == data + b"!"

    for name in [
        "a2b_base64",
        "b2a_base64",
        "a2b_base32",
        "b2a_base32",
        "a2b_base85",
        "b2a_base85",
        "a2b_ascii85",
        "b2a_ascii85",
        "a2b_hex",
        "unhexlify",
    ]:
        fn = getattr(binascii, name)
        try:
            result = str(inspect.signature(fn))
        except ValueError as exc:
            result = "ValueError: " + str(exc)
        print("signature", name, result)
        print("text_signature", name, repr(fn.__text_signature__))


if __name__ == "__main__":
    main()
