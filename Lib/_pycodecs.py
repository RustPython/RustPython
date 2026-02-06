# Note:
# This *is* now explicitly RPython.
# Please make sure not to break this.

# XXX RUSTPYTHON: this was originally from PyPy and has been updated to run on
#                 Python 3. It's currently in the process of being rewritten
#                 to a native Rust module in vm/src/stdlib/codecs.rs

"""

   _codecs -- Provides access to the codec registry and the builtin
              codecs.

   This module should never be imported directly. The standard library
   module "codecs" wraps this builtin module for use within Python.

   The codec registry is accessible via:

     register(search_function) -> None

     lookup(encoding) -> (encoder, decoder, stream_reader, stream_writer)

   The builtin Unicode codecs use the following interface:

     <encoding>_encode(Unicode_object[,errors='strict']) ->
         (string object, bytes consumed)

     <encoding>_decode(char_buffer_obj[,errors='strict']) ->
        (Unicode object, bytes consumed)

   <encoding>_encode() interfaces also accept non-Unicode object as
   input. The objects are then converted to Unicode using
   PyUnicode_FromObject() prior to applying the conversion.

   These <encoding>s are available: utf_8, unicode_escape,
   raw_unicode_escape, unicode_internal, latin_1, ascii (7-bit),
   mbcs (on win32).


Written by Marc-Andre Lemburg (mal@lemburg.com).

Copyright (c) Corporation for National Research Initiatives.

From PyPy v1.0.0

"""
# from unicodecodec import *

__all__ = [
    "register",
    "lookup",
    "lookup_error",
    "register_error",
    "encode",
    "decode",
    "latin_1_encode",
    "mbcs_decode",
    "readbuffer_encode",
    "escape_encode",
    "utf_8_decode",
    "raw_unicode_escape_decode",
    "utf_7_decode",
    "unicode_escape_encode",
    "latin_1_decode",
    "utf_16_decode",
    "unicode_escape_decode",
    "ascii_decode",
    "charmap_encode",
    "charmap_build",
    "unicode_internal_encode",
    "unicode_internal_decode",
    "utf_16_ex_decode",
    "escape_decode",
    "charmap_decode",
    "utf_7_encode",
    "mbcs_encode",
    "ascii_encode",
    "utf_16_encode",
    "raw_unicode_escape_encode",
    "utf_8_encode",
    "utf_16_le_encode",
    "utf_16_be_encode",
    "utf_16_le_decode",
    "utf_16_be_decode",
    "utf_32_ex_decode",
]

import sys
import warnings
from _codecs import *


def latin_1_encode(obj, errors="strict"):
    """None"""
    res = PyUnicode_EncodeLatin1(obj, len(obj), errors)
    res = bytes(res)
    return res, len(obj)


# XXX MBCS codec might involve ctypes ?
def mbcs_decode():
    """None"""
    pass


def readbuffer_encode(obj, errors="strict"):
    """None"""
    if isinstance(obj, str):
        res = obj.encode()
    else:
        res = bytes(obj)
    return res, len(obj)


def escape_encode(obj, errors="strict"):
    """None"""
    if not isinstance(obj, bytes):
        raise TypeError("must be bytes")
    s = repr(obj).encode()
    v = s[2:-1]
    if s[1] == ord('"'):
        v = v.replace(b"'", b"\\'").replace(b'\\"', b'"')
    return v, len(obj)


def raw_unicode_escape_decode(data, errors="strict", final=True):
    """None"""
    res, consumed = PyUnicode_DecodeRawUnicodeEscape(data, len(data), errors, final)
    res = "".join(res)
    return res, consumed


def utf_7_decode(data, errors="strict", final=False):
    """None"""
    res, consumed = PyUnicode_DecodeUTF7(data, len(data), errors, final)
    res = "".join(res)
    return res, consumed


def unicode_escape_encode(obj, errors="strict"):
    """None"""
    res = unicodeescape_string(obj, len(obj), 0)
    res = b"".join(res)
    return res, len(obj)


def latin_1_decode(data, errors="strict"):
    """None"""
    res = PyUnicode_DecodeLatin1(data, len(data), errors)
    res = "".join(res)
    return res, len(data)


def utf_16_decode(data, errors="strict", final=False):
    """None"""
    consumed = len(data)
    if final:
        consumed = 0
    res, consumed, byteorder = PyUnicode_DecodeUTF16Stateful(
        data, len(data), errors, "native", final
    )
    res = "".join(res)
    return res, consumed


def unicode_escape_decode(data, errors="strict", final=True):
    """None"""
    res, consumed = PyUnicode_DecodeUnicodeEscape(data, len(data), errors, final)
    res = "".join(res)
    return res, consumed


def ascii_decode(data, errors="strict"):
    """None"""
    res = PyUnicode_DecodeASCII(data, len(data), errors)
    res = "".join(res)
    return res, len(data)


def charmap_encode(obj, errors="strict", mapping="latin-1"):
    """None"""

    res = PyUnicode_EncodeCharmap(obj, len(obj), mapping, errors)
    res = bytes(res)
    return res, len(obj)


def charmap_build(s):
    return {ord(c): i for i, c in enumerate(s)}


if sys.maxunicode == 65535:
    unicode_bytes = 2
else:
    unicode_bytes = 4


def unicode_internal_encode(obj, errors="strict"):
    """None"""
    if type(obj) == str:
        p = bytearray()
        t = [ord(x) for x in obj]
        for i in t:
            b = bytearray()
            for j in range(unicode_bytes):
                b.append(i % 256)
                i >>= 8
            if sys.byteorder == "big":
                b.reverse()
            p += b
        res = bytes(p)
        return res, len(res)
    else:
        res = "You can do better than this"  # XXX make this right
        return res, len(res)


def unicode_internal_decode(unistr, errors="strict"):
    """None"""
    if type(unistr) == str:
        return unistr, len(unistr)
    else:
        p = []
        i = 0
        if sys.byteorder == "big":
            start = unicode_bytes - 1
            stop = -1
            step = -1
        else:
            start = 0
            stop = unicode_bytes
            step = 1
        while i < len(unistr) - unicode_bytes + 1:
            t = 0
            h = 0
            for j in range(start, stop, step):
                t += ord(unistr[i + j]) << (h * 8)
                h += 1
            i += unicode_bytes
            p += chr(t)
        res = "".join(p)
        return res, len(res)


def utf_16_ex_decode(data, errors="strict", byteorder=0, final=0):
    """None"""
    if byteorder == 0:
        bm = "native"
    elif byteorder == -1:
        bm = "little"
    else:
        bm = "big"
    consumed = len(data)
    if final:
        consumed = 0
    res, consumed, byteorder = PyUnicode_DecodeUTF16Stateful(
        data, len(data), errors, bm, final
    )
    res = "".join(res)
    return res, consumed, byteorder


def utf_32_ex_decode(data, errors="strict", byteorder=0, final=0):
    """None"""
    if byteorder == 0:
        if len(data) < 4:
            if final and len(data):
                if sys.byteorder == "little":
                    bm = "little"
                else:
                    bm = "big"
                res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
                    data, len(data), errors, bm, final
                )
                return "".join(res), consumed, 0
            return "", 0, 0
        if data[0:4] == b"\xff\xfe\x00\x00":
            res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
                data[4:], len(data) - 4, errors, "little", final
            )
            return "".join(res), consumed + 4, -1
        if data[0:4] == b"\x00\x00\xfe\xff":
            res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
                data[4:], len(data) - 4, errors, "big", final
            )
            return "".join(res), consumed + 4, 1
        if sys.byteorder == "little":
            bm = "little"
        else:
            bm = "big"
        res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
            data, len(data), errors, bm, final
        )
        return "".join(res), consumed, 0

    if byteorder == -1:
        res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
            data, len(data), errors, "little", final
        )
        return "".join(res), consumed, -1

    res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
        data, len(data), errors, "big", final
    )
    return "".join(res), consumed, 1


def _is_hex_digit(b):
    return (
        0x30 <= b <= 0x39  # 0-9
        or 0x41 <= b <= 0x46  # A-F
        or 0x61 <= b <= 0x66
    )  # a-f


def escape_decode(data, errors="strict"):
    if isinstance(data, str):
        data = data.encode("latin-1")
    l = len(data)
    i = 0
    res = bytearray()
    while i < l:
        if data[i] == 0x5C:  # '\\'
            i += 1
            if i >= l:
                raise ValueError("Trailing \\ in string")
            ch = data[i]
            if ch == 0x5C:
                res.append(0x5C)  # \\
            elif ch == 0x27:
                res.append(0x27)  # \'
            elif ch == 0x22:
                res.append(0x22)  # \"
            elif ch == 0x61:
                res.append(0x07)  # \a
            elif ch == 0x62:
                res.append(0x08)  # \b
            elif ch == 0x66:
                res.append(0x0C)  # \f
            elif ch == 0x6E:
                res.append(0x0A)  # \n
            elif ch == 0x72:
                res.append(0x0D)  # \r
            elif ch == 0x74:
                res.append(0x09)  # \t
            elif ch == 0x76:
                res.append(0x0B)  # \v
            elif ch == 0x0A:
                pass  # \<newline> continuation
            elif 0x30 <= ch <= 0x37:  # \0-\7 octal
                val = ch - 0x30
                if i + 1 < l and 0x30 <= data[i + 1] <= 0x37:
                    i += 1
                    val = (val << 3) | (data[i] - 0x30)
                    if i + 1 < l and 0x30 <= data[i + 1] <= 0x37:
                        i += 1
                        val = (val << 3) | (data[i] - 0x30)
                res.append(val & 0xFF)
            elif ch == 0x78:  # \x hex
                hex_count = 0
                for j in range(1, 3):
                    if i + j < l and _is_hex_digit(data[i + j]):
                        hex_count += 1
                    else:
                        break
                if hex_count < 2:
                    if errors == "strict":
                        raise ValueError("invalid \\x escape at position %d" % (i - 1))
                    elif errors == "replace":
                        res.append(0x3F)  # '?'
                    i += hex_count
                else:
                    res.append(int(bytes(data[i + 1 : i + 3]), 16))
                    i += 2
            else:
                import warnings

                warnings.warn(
                    '"\\%c" is an invalid escape sequence' % ch
                    if 0x20 <= ch < 0x7F
                    else '"\\x%02x" is an invalid escape sequence' % ch,
                    DeprecationWarning,
                    stacklevel=2,
                )
                res.append(0x5C)
                res.append(ch)
        else:
            res.append(data[i])
        i += 1
    return bytes(res), l


def charmap_decode(data, errors="strict", mapping=None):
    """None"""
    res = PyUnicode_DecodeCharmap(data, len(data), mapping, errors)
    res = "".join(res)
    return res, len(data)


def utf_7_encode(obj, errors="strict"):
    """None"""
    res = PyUnicode_EncodeUTF7(obj, len(obj), 0, 0, errors)
    res = b"".join(res)
    return res, len(obj)


def mbcs_encode(obj, errors="strict"):
    """None"""
    pass


##    return (PyUnicode_EncodeMBCS(
##                             (obj),
##                             len(obj),
##                             errors),
##                  len(obj))


def ascii_encode(obj, errors="strict"):
    """None"""
    res = PyUnicode_EncodeASCII(obj, len(obj), errors)
    res = bytes(res)
    return res, len(obj)


def utf_16_encode(obj, errors="strict"):
    """None"""
    res = PyUnicode_EncodeUTF16(obj, len(obj), errors, "native")
    res = bytes(res)
    return res, len(obj)


def raw_unicode_escape_encode(obj, errors="strict"):
    """None"""
    res = PyUnicode_EncodeRawUnicodeEscape(obj, len(obj))
    res = bytes(res)
    return res, len(obj)


def utf_16_le_encode(obj, errors="strict"):
    """None"""
    res = PyUnicode_EncodeUTF16(obj, len(obj), errors, "little")
    res = bytes(res)
    return res, len(obj)


def utf_16_be_encode(obj, errors="strict"):
    """None"""
    res = PyUnicode_EncodeUTF16(obj, len(obj), errors, "big")
    res = bytes(res)
    return res, len(obj)


def utf_16_le_decode(data, errors="strict", final=0):
    res, consumed, byteorder = PyUnicode_DecodeUTF16Stateful(
        data, len(data), errors, "little", final
    )
    res = "".join(res)
    return res, consumed


def utf_16_be_decode(data, errors="strict", final=0):
    res, consumed, byteorder = PyUnicode_DecodeUTF16Stateful(
        data, len(data), errors, "big", final
    )
    res = "".join(res)
    return res, consumed


def STORECHAR32(ch, byteorder):
    """Store a 32-bit character as 4 bytes in the specified byte order."""
    b0 = ch & 0xFF
    b1 = (ch >> 8) & 0xFF
    b2 = (ch >> 16) & 0xFF
    b3 = (ch >> 24) & 0xFF
    if byteorder == "little":
        return [b0, b1, b2, b3]
    else:  # big-endian
        return [b3, b2, b1, b0]


def PyUnicode_EncodeUTF32(s, size, errors, byteorder="little"):
    """Encode a Unicode string to UTF-32."""
    p = []
    bom = sys.byteorder

    if byteorder == "native":
        bom = sys.byteorder
        # Add BOM for native encoding
        p += STORECHAR32(0xFEFF, bom)

    if byteorder == "little":
        bom = "little"
    elif byteorder == "big":
        bom = "big"

    pos = 0
    while pos < len(s):
        ch = ord(s[pos])
        if 0xD800 <= ch <= 0xDFFF:
            if errors == "surrogatepass":
                p += STORECHAR32(ch, bom)
                pos += 1
            else:
                res, pos = unicode_call_errorhandler(
                    errors, "utf-32", "surrogates not allowed", s, pos, pos + 1, False
                )
                for c in res:
                    p += STORECHAR32(ord(c), bom)
        else:
            p += STORECHAR32(ch, bom)
            pos += 1

    return p


def utf_32_encode(obj, errors="strict"):
    """UTF-32 encoding with BOM."""
    encoded = PyUnicode_EncodeUTF32(obj, len(obj), errors, "native")
    return bytes(encoded), len(obj)


def utf_32_le_encode(obj, errors="strict"):
    """UTF-32 little-endian encoding without BOM."""
    encoded = PyUnicode_EncodeUTF32(obj, len(obj), errors, "little")
    return bytes(encoded), len(obj)


def utf_32_be_encode(obj, errors="strict"):
    """UTF-32 big-endian encoding without BOM."""
    res = PyUnicode_EncodeUTF32(obj, len(obj), errors, "big")
    res = bytes(res)
    return res, len(obj)


def PyUnicode_DecodeUTF32Stateful(data, size, errors, byteorder="little", final=0):
    """Decode UTF-32 encoded bytes to Unicode string."""
    if size == 0:
        return [], 0, 0

    result = []
    pos = 0
    aligned_size = (size // 4) * 4

    while pos + 3 < aligned_size:
        if byteorder == "little":
            ch = (
                data[pos]
                | (data[pos + 1] << 8)
                | (data[pos + 2] << 16)
                | (data[pos + 3] << 24)
            )
        else:  # big-endian
            ch = (
                (data[pos] << 24)
                | (data[pos + 1] << 16)
                | (data[pos + 2] << 8)
                | data[pos + 3]
            )

        # Validate code point
        if ch > 0x10FFFF:
            if errors == "strict":
                raise UnicodeDecodeError(
                    "utf-32",
                    bytes(data),
                    pos,
                    pos + 4,
                    "codepoint not in range(0x110000)",
                )
            elif errors == "replace":
                result.append("\ufffd")
            # 'ignore' - skip this character
            pos += 4
        elif 0xD800 <= ch <= 0xDFFF:
            if errors == "surrogatepass":
                result.append(chr(ch))
                pos += 4
            else:
                msg = "code point in surrogate code point range(0xd800, 0xe000)"
                res, pos = unicode_call_errorhandler(
                    errors, "utf-32", msg, data, pos, pos + 4, True
                )
                result.append(res)
        else:
            result.append(chr(ch))
            pos += 4

    # Handle trailing incomplete bytes
    if pos < size:
        if final:
            res, pos = unicode_call_errorhandler(
                errors, "utf-32", "truncated data", data, pos, size, True
            )
            if res:
                result.append(res)

    return result, pos, 0


def utf_32_decode(data, errors="strict", final=0):
    """UTF-32 decoding with BOM detection."""
    if len(data) >= 4:
        # Check for BOM
        if data[0:4] == b"\xff\xfe\x00\x00":
            # UTF-32 LE BOM
            res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
                data[4:], len(data) - 4, errors, "little", final
            )
            res = "".join(res)
            return res, consumed + 4
        elif data[0:4] == b"\x00\x00\xfe\xff":
            # UTF-32 BE BOM
            res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
                data[4:], len(data) - 4, errors, "big", final
            )
            res = "".join(res)
            return res, consumed + 4

    # Default to little-endian if no BOM
    byteorder = "little" if sys.byteorder == "little" else "big"
    res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
        data, len(data), errors, byteorder, final
    )
    res = "".join(res)
    return res, consumed


def utf_32_le_decode(data, errors="strict", final=0):
    """UTF-32 little-endian decoding without BOM."""
    res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
        data, len(data), errors, "little", final
    )
    res = "".join(res)
    return res, consumed


def utf_32_be_decode(data, errors="strict", final=0):
    """UTF-32 big-endian decoding without BOM."""
    res, consumed, _ = PyUnicode_DecodeUTF32Stateful(
        data, len(data), errors, "big", final
    )
    res = "".join(res)
    return res, consumed


#  ----------------------------------------------------------------------

##import sys
##""" Python implementation of CPythons builtin unicode codecs.
##
##    Generally the functions in this module take a list of characters an returns
##    a list of characters.
##
##    For use in the PyPy project"""


## indicate whether a UTF-7 character is special i.e. cannot be directly
##       encoded:
##         0 - not special
##         1 - special
##         2 - whitespace (optional)
##         3 - RFC2152 Set O (optional)

utf7_special = [
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    2,
    2,
    1,
    1,
    2,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    2,
    3,
    3,
    3,
    3,
    3,
    3,
    0,
    0,
    0,
    3,
    1,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    3,
    3,
    3,
    3,
    0,
    3,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    3,
    1,
    3,
    3,
    3,
    3,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    3,
    3,
    3,
    1,
    1,
]
unicode_latin1 = [None] * 256


def SPECIAL(c, encodeO, encodeWS):
    c = ord(c)
    return (
        (c > 127 or utf7_special[c] == 1)
        or (encodeWS and (utf7_special[(c)] == 2))
        or (encodeO and (utf7_special[(c)] == 3))
    )


def B64(n):
    return bytes(
        [
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"[
                (n) & 0x3F
            ]
        ]
    )


def B64CHAR(c):
    return c.isalnum() or (c) == b"+" or (c) == b"/"


def UB64(c):
    if (c) == b"+":
        return 62
    elif (c) == b"/":
        return 63
    elif (c) >= b"a":
        return ord(c) - 71
    elif (c) >= b"A":
        return ord(c) - 65
    else:
        return ord(c) + 4


def ENCODE(ch, bits):
    out = []
    while bits >= 6:
        out += B64(ch >> (bits - 6))
        bits -= 6
    return out, bits


def _IS_BASE64(ch):
    return (
        (ord("A") <= ch <= ord("Z"))
        or (ord("a") <= ch <= ord("z"))
        or (ord("0") <= ch <= ord("9"))
        or ch == ord("+")
        or ch == ord("/")
    )


def _FROM_BASE64(ch):
    if ch == ord("+"):
        return 62
    if ch == ord("/"):
        return 63
    if ch >= ord("a"):
        return ch - 71
    if ch >= ord("A"):
        return ch - 65
    if ch >= ord("0"):
        return ch - ord("0") + 52
    return -1


def _DECODE_DIRECT(ch):
    return ch <= 127 and ch != ord("+")


def PyUnicode_DecodeUTF7(s, size, errors, final=False):
    if size == 0:
        return [], 0

    p = []
    inShift = False
    base64bits = 0
    base64buffer = 0
    surrogate = 0
    startinpos = 0
    shiftOutStart = 0
    i = 0

    while i < size:
        ch = s[i]
        if inShift:
            if _IS_BASE64(ch):
                base64buffer = (base64buffer << 6) | _FROM_BASE64(ch)
                base64bits += 6
                i += 1
                if base64bits >= 16:
                    outCh = (base64buffer >> (base64bits - 16)) & 0xFFFF
                    base64bits -= 16
                    base64buffer &= (1 << base64bits) - 1
                    if surrogate:
                        if 0xDC00 <= outCh <= 0xDFFF:
                            ch2 = (
                                0x10000
                                + ((surrogate - 0xD800) << 10)
                                + (outCh - 0xDC00)
                            )
                            p.append(chr(ch2))
                            surrogate = 0
                            continue
                        else:
                            p.append(chr(surrogate))
                            surrogate = 0
                    if 0xD800 <= outCh <= 0xDBFF:
                        surrogate = outCh
                    else:
                        p.append(chr(outCh))
            else:
                inShift = False
                if base64bits > 0:
                    if base64bits >= 6:
                        i += 1
                        errmsg = "partial character in shift sequence"
                        out, i = unicode_call_errorhandler(
                            errors, "utf-7", errmsg, s, startinpos, i
                        )
                        p.append(out)
                        continue
                    else:
                        if base64buffer != 0:
                            i += 1
                            errmsg = "non-zero padding bits in shift sequence"
                            out, i = unicode_call_errorhandler(
                                errors, "utf-7", errmsg, s, startinpos, i
                            )
                            p.append(out)
                            continue
                if surrogate and _DECODE_DIRECT(ch):
                    p.append(chr(surrogate))
                surrogate = 0
                if ch == ord("-"):
                    i += 1
        elif ch == ord("+"):
            startinpos = i
            i += 1
            if i < size and s[i] == ord("-"):
                i += 1
                p.append("+")
            elif i < size and not _IS_BASE64(s[i]):
                i += 1
                errmsg = "ill-formed sequence"
                out, i = unicode_call_errorhandler(
                    errors, "utf-7", errmsg, s, startinpos, i
                )
                p.append(out)
            else:
                inShift = True
                surrogate = 0
                shiftOutStart = len(p)
                base64bits = 0
                base64buffer = 0
        elif _DECODE_DIRECT(ch):
            i += 1
            p.append(chr(ch))
        else:
            startinpos = i
            i += 1
            errmsg = "unexpected special character"
            out, i = unicode_call_errorhandler(
                errors, "utf-7", errmsg, s, startinpos, i
            )
            p.append(out)

    if inShift and not final:
        return p[:shiftOutStart], startinpos

    if inShift and final:
        if surrogate or base64bits >= 6 or (base64bits > 0 and base64buffer != 0):
            errmsg = "unterminated shift sequence"
            out, i = unicode_call_errorhandler(
                errors, "utf-7", errmsg, s, startinpos, size
            )
            p.append(out)

    return p, size


def _ENCODE_DIRECT(ch, encodeSetO, encodeWhiteSpace):
    c = ord(ch) if isinstance(ch, str) else ch
    if c > 127:
        return False
    if utf7_special[c] == 0:
        return True
    if utf7_special[c] == 2:
        return not encodeWhiteSpace
    if utf7_special[c] == 3:
        return not encodeSetO
    return False


def PyUnicode_EncodeUTF7(s, size, encodeSetO, encodeWhiteSpace, errors):
    inShift = False
    base64bits = 0
    base64buffer = 0
    out = []

    for i, ch in enumerate(s):
        ch_ord = ord(ch)
        if inShift:
            if _ENCODE_DIRECT(ch, encodeSetO, encodeWhiteSpace):
                # shifting out
                if base64bits:
                    out.append(B64(base64buffer << (6 - base64bits)))
                    base64buffer = 0
                    base64bits = 0
                inShift = False
                if B64CHAR(ch) or ch == "-":
                    out.append(b"-")
                out.append(bytes([ch_ord]))
            else:
                # encode character in base64
                if ch_ord >= 0x10000:
                    # split into surrogate pair
                    hi = 0xD800 | ((ch_ord - 0x10000) >> 10)
                    lo = 0xDC00 | ((ch_ord - 0x10000) & 0x3FF)
                    base64bits += 16
                    base64buffer = (base64buffer << 16) | hi
                    while base64bits >= 6:
                        out.append(B64(base64buffer >> (base64bits - 6)))
                        base64bits -= 6
                    base64buffer &= (1 << base64bits) - 1 if base64bits else 0
                    ch_ord = lo

                base64bits += 16
                base64buffer = (base64buffer << 16) | ch_ord
                while base64bits >= 6:
                    out.append(B64(base64buffer >> (base64bits - 6)))
                    base64bits -= 6
                base64buffer &= (1 << base64bits) - 1 if base64bits else 0
        else:
            if ch == "+":
                out.append(b"+-")
            elif _ENCODE_DIRECT(ch, encodeSetO, encodeWhiteSpace):
                out.append(bytes([ch_ord]))
            else:
                out.append(b"+")
                inShift = True
                # encode character in base64
                if ch_ord >= 0x10000:
                    hi = 0xD800 | ((ch_ord - 0x10000) >> 10)
                    lo = 0xDC00 | ((ch_ord - 0x10000) & 0x3FF)
                    base64bits += 16
                    base64buffer = (base64buffer << 16) | hi
                    while base64bits >= 6:
                        out.append(B64(base64buffer >> (base64bits - 6)))
                        base64bits -= 6
                    base64buffer &= (1 << base64bits) - 1 if base64bits else 0
                    ch_ord = lo

                base64bits += 16
                base64buffer = (base64buffer << 16) | ch_ord
                while base64bits >= 6:
                    out.append(B64(base64buffer >> (base64bits - 6)))
                    base64bits -= 6
                base64buffer &= (1 << base64bits) - 1 if base64bits else 0

                if base64bits == 0:
                    if i + 1 < size:
                        ch2 = s[i + 1]
                        if _ENCODE_DIRECT(ch2, encodeSetO, encodeWhiteSpace):
                            if B64CHAR(ch2) or ch2 == "-":
                                out.append(b"-")
                            inShift = False
                    else:
                        out.append(b"-")
                        inShift = False

    if base64bits:
        out.append(B64(base64buffer << (6 - base64bits)))
    if inShift:
        out.append(b"-")

    return out


unicode_empty = ""


def unicodeescape_string(s, size, quotes):
    p = []
    if quotes:
        if s.find("'") != -1 and s.find('"') == -1:
            p.append(b'"')
        else:
            p.append(b"'")
    pos = 0
    while pos < size:
        ch = s[pos]
        # /* Escape quotes */
        if quotes and (ch == p[1] or ch == "\\"):
            p.append(b"\\%c" % ord(ch))
            pos += 1
            continue

        # ifdef Py_UNICODE_WIDE
        # /* Map 21-bit characters to '\U00xxxxxx' */
        elif ord(ch) >= 0x10000:
            p.append(b"\\U%08x" % ord(ch))
            pos += 1
            continue
        # endif
        # /* Map UTF-16 surrogate pairs to Unicode \UXXXXXXXX escapes */
        elif ord(ch) >= 0xD800 and ord(ch) < 0xDC00:
            pos += 1
            ch2 = s[pos]

            if ord(ch2) >= 0xDC00 and ord(ch2) <= 0xDFFF:
                ucs = (((ord(ch) & 0x03FF) << 10) | (ord(ch2) & 0x03FF)) + 0x00010000
                p.append(b"\\U%08x" % ucs)
                pos += 1
                continue

            # /* Fall through: isolated surrogates are copied as-is */
            pos -= 1

        # /* Map 16-bit characters to '\uxxxx' */
        if ord(ch) >= 256:
            p.append(b"\\u%04x" % ord(ch))

        # /* Map special whitespace to '\t', \n', '\r' */
        elif ch == "\t":
            p.append(b"\\t")

        elif ch == "\n":
            p.append(b"\\n")

        elif ch == "\r":
            p.append(b"\\r")

        elif ch == "\\":
            p.append(b"\\\\")

        # /* Map non-printable US ASCII to '\xhh' */
        elif ch < " " or ch >= chr(0x7F):
            p.append(b"\\x%02x" % ord(ch))
        # /* Copy everything else as-is */
        else:
            p.append(bytes([ord(ch)]))
        pos += 1
    if quotes:
        p.append(p[0])
    return p


def PyUnicode_DecodeASCII(s, size, errors):
    #    /* ASCII is equivalent to the first 128 ordinals in Unicode. */
    if size == 1 and ord(s) < 128:
        return [chr(ord(s))]
    if size == 0:
        return [""]  # unicode('')
    p = []
    pos = 0
    while pos < len(s):
        c = s[pos]
        if c < 128:
            p += chr(c)
            pos += 1
        else:
            res = unicode_call_errorhandler(
                errors, "ascii", "ordinal not in range(128)", s, pos, pos + 1
            )
            p += res[0]
            pos = res[1]
    return p


def PyUnicode_EncodeASCII(p, size, errors):
    return unicode_encode_ucs1(p, size, errors, 128)


def PyUnicode_AsASCIIString(unistr):
    if not type(unistr) == str:
        raise TypeError
    return PyUnicode_EncodeASCII(unistr, len(unistr), None)


def PyUnicode_DecodeUTF16Stateful(s, size, errors, byteorder="native", final=True):
    bo = 0  # /* assume native ordering by default */
    consumed = 0
    errmsg = ""

    if sys.byteorder == "little":
        ihi = 1
        ilo = 0
    else:
        ihi = 0
        ilo = 1

    # /* Unpack UTF-16 encoded data */

    ##    /* Check for BOM marks (U+FEFF) in the input and adjust current
    ##       byte order setting accordingly. In native mode, the leading BOM
    ##       mark is skipped, in all other modes, it is copied to the output
    ##       stream as-is (giving a ZWNBSP character). */
    q = 0
    p = []
    if byteorder == "native":
        if size >= 2:
            bom = (s[ihi] << 8) | s[ilo]
            # ifdef BYTEORDER_IS_LITTLE_ENDIAN
            if sys.byteorder == "little":
                if bom == 0xFEFF:
                    q += 2
                    bo = -1
                elif bom == 0xFFFE:
                    q += 2
                    bo = 1
            else:
                if bom == 0xFEFF:
                    q += 2
                    bo = 1
                elif bom == 0xFFFE:
                    q += 2
                    bo = -1
    elif byteorder == "little":
        bo = -1
    else:
        bo = 1

    if size == 0:
        return [""], 0, bo

    if bo == -1:
        # /* force LE */
        ihi = 1
        ilo = 0

    elif bo == 1:
        # /* force BE */
        ihi = 0
        ilo = 1

    while q < len(s):
        # /* remaining bytes at the end? (size should be even) */
        if len(s) - q < 2:
            if not final:
                break
            res, q = unicode_call_errorhandler(
                errors, "utf-16", "truncated data", s, q, len(s), True
            )
            p.append(res)
            break

        ch = (s[q + ihi] << 8) | s[q + ilo]

        if ch < 0xD800 or ch > 0xDFFF:
            p.append(chr(ch))
            q += 2
            continue

        # /* UTF-16 code pair: high surrogate */
        if 0xD800 <= ch <= 0xDBFF:
            if q + 4 <= len(s):
                ch2 = (s[q + 2 + ihi] << 8) | s[q + 2 + ilo]
                if 0xDC00 <= ch2 <= 0xDFFF:
                    # Valid surrogate pair - always assemble
                    p.append(chr((((ch & 0x3FF) << 10) | (ch2 & 0x3FF)) + 0x10000))
                    q += 4
                    continue
                else:
                    # High surrogate followed by non-low-surrogate
                    if errors == "surrogatepass":
                        p.append(chr(ch))
                        q += 2
                        continue
                    res, q = unicode_call_errorhandler(
                        errors, "utf-16", "illegal UTF-16 surrogate", s, q, q + 2, True
                    )
                    p.append(res)
            else:
                # High surrogate at end of data
                if not final:
                    break
                if errors == "surrogatepass":
                    p.append(chr(ch))
                    q += 2
                    continue
                res, q = unicode_call_errorhandler(
                    errors, "utf-16", "unexpected end of data", s, q, len(s), True
                )
                p.append(res)
        else:
            # Low surrogate without preceding high surrogate
            if errors == "surrogatepass":
                p.append(chr(ch))
                q += 2
                continue
            res, q = unicode_call_errorhandler(
                errors, "utf-16", "illegal encoding", s, q, q + 2, True
            )
            p.append(res)

    return p, q, bo


# moved out of local scope, especially because it didn't
# have any nested variables.


def STORECHAR(CH, byteorder):
    hi = (CH >> 8) & 0xFF
    lo = CH & 0xFF
    if byteorder == "little":
        return [lo, hi]
    else:
        return [hi, lo]


def PyUnicode_EncodeUTF16(s, size, errors, byteorder="little"):
    #    /* Offsets from p for storing byte pairs in the right order. */

    p = []
    bom = sys.byteorder
    if byteorder == "native":
        bom = sys.byteorder
        p += STORECHAR(0xFEFF, bom)

    if byteorder == "little":
        bom = "little"
    elif byteorder == "big":
        bom = "big"

    pos = 0
    while pos < len(s):
        ch = ord(s[pos])
        if 0xD800 <= ch <= 0xDFFF:
            if errors == "surrogatepass":
                p += STORECHAR(ch, bom)
                pos += 1
            else:
                res, pos = unicode_call_errorhandler(
                    errors, "utf-16", "surrogates not allowed", s, pos, pos + 1, False
                )
                for c in res:
                    cp = ord(c)
                    cp2 = 0
                    if cp >= 0x10000:
                        cp2 = 0xDC00 | ((cp - 0x10000) & 0x3FF)
                        cp = 0xD800 | ((cp - 0x10000) >> 10)
                    p += STORECHAR(cp, bom)
                    if cp2:
                        p += STORECHAR(cp2, bom)
        else:
            ch2 = 0
            if ch >= 0x10000:
                ch2 = 0xDC00 | ((ch - 0x10000) & 0x3FF)
                ch = 0xD800 | ((ch - 0x10000) >> 10)
            p += STORECHAR(ch, bom)
            if ch2:
                p += STORECHAR(ch2, bom)
            pos += 1

    return p


def PyUnicode_DecodeMBCS(s, size, errors):
    pass


def PyUnicode_EncodeMBCS(p, size, errors):
    pass


def unicode_call_errorhandler(
    errors, encoding, reason, input, startinpos, endinpos, decode=True
):
    errorHandler = lookup_error(errors)
    if decode:
        exceptionObject = UnicodeDecodeError(
            encoding, input, startinpos, endinpos, reason
        )
    else:
        exceptionObject = UnicodeEncodeError(
            encoding, input, startinpos, endinpos, reason
        )
    res = errorHandler(exceptionObject)
    if (
        isinstance(res, tuple)
        and isinstance(res[0], (str, bytes))
        and isinstance(res[1], int)
    ):
        newpos = res[1]
        if newpos < 0:
            newpos = len(input) + newpos
        if newpos < 0 or newpos > len(input):
            raise IndexError("position %d from error handler out of bounds" % newpos)
        return res[0], newpos
    else:
        raise TypeError(
            "encoding error handler must return (unicode, int) tuple, not %s"
            % repr(res)
        )


# /* --- Latin-1 Codec ------------------------------------------------------ */


def PyUnicode_DecodeLatin1(s, size, errors):
    # /* Latin-1 is equivalent to the first 256 ordinals in Unicode. */
    ##    if (size == 1):
    ##        return [PyUnicode_FromUnicode(s, 1)]
    pos = 0
    p = []
    while pos < size:
        p += chr(s[pos])
        pos += 1
    return p


def unicode_encode_ucs1(p, size, errors, limit):
    if limit == 256:
        reason = "ordinal not in range(256)"
        encoding = "latin-1"
    else:
        reason = "ordinal not in range(128)"
        encoding = "ascii"

    if size == 0:
        return []
    res = bytearray()
    pos = 0
    while pos < len(p):
        # for ch in p:
        ch = p[pos]

        if ord(ch) < limit:
            res.append(ord(ch))
            pos += 1
        else:
            # /* startpos for collecting unencodable chars */
            collstart = pos
            collend = pos + 1
            while collend < len(p) and ord(p[collend]) >= limit:
                collend += 1
            x = unicode_call_errorhandler(
                errors, encoding, reason, p, collstart, collend, False
            )
            replacement = x[0]
            if isinstance(replacement, bytes):
                res += replacement
            else:
                res += replacement.encode()
            pos = x[1]

    return res


def PyUnicode_EncodeLatin1(p, size, errors):
    res = unicode_encode_ucs1(p, size, errors, 256)
    return res


hexdigits = [ord(hex(i)[-1]) for i in range(16)] + [
    ord(hex(i)[-1].upper()) for i in range(10, 16)
]


def hex_number_end(s, pos, digits):
    target_end = pos + digits
    while pos < target_end and pos < len(s) and s[pos] in hexdigits:
        pos += 1
    return pos


def hexescape(s, pos, digits, message, errors):
    ch = 0
    p = []
    number_end = hex_number_end(s, pos, digits)
    if number_end - pos != digits:
        x = unicode_call_errorhandler(
            errors, "unicodeescape", message, s, pos - 2, number_end
        )
        p.append(x[0])
        pos = x[1]
    else:
        ch = int(s[pos : pos + digits], 16)
        # /* when we get here, ch is a 32-bit unicode character */
        if ch <= sys.maxunicode:
            p.append(chr(ch))
            pos += digits

        elif ch <= 0x10FFFF:
            ch -= 0x10000
            p.append(chr(0xD800 + (ch >> 10)))
            p.append(chr(0xDC00 + (ch & 0x03FF)))
            pos += digits
        else:
            message = "illegal Unicode character"
            x = unicode_call_errorhandler(
                errors, "unicodeescape", message, s, pos - 2, pos + digits
            )
            p.append(x[0])
            pos = x[1]
    res = p
    return res, pos


def PyUnicode_DecodeUnicodeEscape(s, size, errors, final):
    if size == 0:
        return "", 0

    if isinstance(s, str):
        s = s.encode()

    found_invalid_escape = False

    p = []
    pos = 0
    while pos < size:
        ##        /* Non-escape characters are interpreted as Unicode ordinals */
        if s[pos] != ord("\\"):
            p.append(chr(s[pos]))
            pos += 1
            continue
        ##        /* \ - Escapes */
        escape_start = pos
        pos += 1
        if pos >= size:
            if not final:
                pos = escape_start
                break
            errmessage = "\\ at end of string"
            unicode_call_errorhandler(
                errors, "unicodeescape", errmessage, s, pos - 1, size
            )
            break
        ch = chr(s[pos])
        pos += 1
        ##        /* \x escapes */
        if ch == "\n":
            pass
        elif ch == "\\":
            p += "\\"
        elif ch == "'":
            p += "'"
        elif ch == '"':
            p += '"'
        elif ch == "b":
            p += "\b"
        elif ch == "f":
            p += "\014"  # /* FF */
        elif ch == "t":
            p += "\t"
        elif ch == "n":
            p += "\n"
        elif ch == "r":
            p += "\r"
        elif ch == "v":
            p += "\013"  # break; /* VT */
        elif ch == "a":
            p += "\007"  # break; /* BEL, not classic C */
        elif "0" <= ch <= "7":
            x = ord(ch) - ord("0")
            if pos < size:
                ch = chr(s[pos])
                if "0" <= ch <= "7":
                    pos += 1
                    x = (x << 3) + ord(ch) - ord("0")
                    if pos < size:
                        ch = chr(s[pos])
                        if "0" <= ch <= "7":
                            pos += 1
                            x = (x << 3) + ord(ch) - ord("0")
            p.append(chr(x))
        ##        /* hex escapes */
        ##        /* \xXX */
        elif ch in ("x", "u", "U"):
            if ch == "x":
                digits = 2
                message = "truncated \\xXX escape"
            elif ch == "u":
                digits = 4
                message = "truncated \\uXXXX escape"
            else:
                digits = 8
                message = "truncated \\UXXXXXXXX escape"
            number_end = hex_number_end(s, pos, digits)
            if number_end - pos != digits:
                if not final:
                    pos = escape_start
                    break
                x = hexescape(s, pos, digits, message, errors)
                p += x[0]
                pos = x[1]
            else:
                x = hexescape(s, pos, digits, message, errors)
                p += x[0]
                pos = x[1]
        ##        /* \N{name} */
        elif ch == "N":
            message = "malformed \\N character escape"
            look = pos
            try:
                import unicodedata
            except ImportError:
                message = "\\N escapes not supported (can't load unicodedata module)"
                unicode_call_errorhandler(
                    errors, "unicodeescape", message, s, pos - 1, size
                )
                continue
            if look < size and chr(s[look]) == "{":
                # /* look for the closing brace */
                while look < size and chr(s[look]) != "}":
                    look += 1
                if look > pos + 1 and look < size and chr(s[look]) == "}":
                    # /* found a name.  look it up in the unicode database */
                    message = "unknown Unicode character name"
                    st = s[pos + 1 : look]
                    try:
                        chr_codec = unicodedata.lookup("%s" % st)
                    except LookupError as e:
                        x = unicode_call_errorhandler(
                            errors, "unicodeescape", message, s, pos - 1, look + 1
                        )
                    else:
                        x = chr_codec, look + 1
                    p.append(x[0])
                    pos = x[1]
                else:
                    if not final:
                        pos = escape_start
                        break
                    x = unicode_call_errorhandler(
                        errors, "unicodeescape", message, s, pos - 1, look + 1
                    )
                    p.append(x[0])
                    pos = x[1]
            else:
                if not final:
                    pos = escape_start
                    break
                x = unicode_call_errorhandler(
                    errors, "unicodeescape", message, s, pos - 1, look + 1
                )
                p.append(x[0])
                pos = x[1]
        else:
            if not found_invalid_escape:
                found_invalid_escape = True
                warnings.warn(
                    "invalid escape sequence '\\%c'" % ch, DeprecationWarning, 2
                )
            p.append("\\")
            p.append(ch)
    return p, pos


def PyUnicode_EncodeRawUnicodeEscape(s, size):
    if size == 0:
        return b""

    p = bytearray()
    for ch in s:
        #       /* Map 32-bit characters to '\Uxxxxxxxx' */
        if ord(ch) >= 0x10000:
            p += b"\\U%08x" % ord(ch)
        elif ord(ch) >= 256:
            #       /* Map 16-bit characters to '\uxxxx' */
            p += b"\\u%04x" % (ord(ch))
        #       /* Copy everything else as-is */
        else:
            p.append(ord(ch))

    # p += '\0'
    return p


def charmapencode_output(c, mapping):
    rep = mapping[c]
    if isinstance(rep, int):
        if rep < 256:
            return [rep]
        else:
            raise TypeError("character mapping must be in range(256)")
    elif isinstance(rep, str):
        return [ord(rep)]
    elif isinstance(rep, bytes):
        return rep
    elif rep == None:
        raise KeyError("character maps to <undefined>")
    else:
        raise TypeError("character mapping must return integer, None or str")


def PyUnicode_EncodeCharmap(p, size, mapping="latin-1", errors="strict"):
    ##    /* the following variable is used for caching string comparisons
    ##     * -1=not initialized, 0=unknown, 1=strict, 2=replace,
    ##     * 3=ignore, 4=xmlcharrefreplace */

    #    /* Default to Latin-1 */
    if mapping == "latin-1":
        return PyUnicode_EncodeLatin1(p, size, errors)
    if size == 0:
        return b""
    inpos = 0
    res = []
    while inpos < size:
        # /* try to encode it */
        try:
            x = charmapencode_output(ord(p[inpos]), mapping)
            res += x
        except KeyError:
            x = unicode_call_errorhandler(
                errors,
                "charmap",
                "character maps to <undefined>",
                p,
                inpos,
                inpos + 1,
                False,
            )
            replacement = x[0]
            if isinstance(replacement, bytes):
                res += list(replacement)
            else:
                try:
                    for y in replacement:
                        res += charmapencode_output(ord(y), mapping)
                except KeyError:
                    raise UnicodeEncodeError(
                        "charmap", p, inpos, inpos + 1, "character maps to <undefined>"
                    )
        inpos += 1
    return res


def PyUnicode_DecodeCharmap(s, size, mapping, errors):
    ##    /* Default to Latin-1 */
    if mapping == None:
        return PyUnicode_DecodeLatin1(s, size, errors)

    if size == 0:
        return ""
    p = []
    inpos = 0
    while inpos < len(s):
        # /* Get mapping (char ordinal -> integer, Unicode char or None) */
        ch = s[inpos]
        try:
            x = mapping[ch]
            if isinstance(x, int):
                if x == 0xFFFE:
                    raise KeyError
                if 0 <= x <= 0x10FFFF:
                    p += chr(x)
                else:
                    raise TypeError(
                        "character mapping must be in range(0x%x)" % (0x110000,)
                    )
            elif isinstance(x, str):
                if len(x) == 1 and x == "\ufffe":
                    raise KeyError
                p += x
            elif x is None:
                raise KeyError
            else:
                raise TypeError
        except (KeyError, IndexError):
            x = unicode_call_errorhandler(
                errors, "charmap", "character maps to <undefined>", s, inpos, inpos + 1
            )
            p += x[0]
        inpos += 1
    return p


def PyUnicode_DecodeRawUnicodeEscape(s, size, errors, final):
    if size == 0:
        return "", 0

    if isinstance(s, str):
        s = s.encode()

    pos = 0
    p = []
    while pos < len(s):
        # /* Non-escape characters are interpreted as Unicode ordinals */
        if s[pos] != ord("\\"):
            p.append(chr(s[pos]))
            pos += 1
            continue
        startinpos = pos
        p_len_before = len(p)
        ##      /* \u-escapes are only interpreted iff the number of leading
        ##         backslashes is odd */
        bs = pos
        while pos < size:
            if s[pos] != ord("\\"):
                break
            p.append(chr(s[pos]))
            pos += 1

        if pos >= size:
            if not final:
                del p[p_len_before:]
                pos = startinpos
            break
        if ((pos - bs) & 1) == 0 or (s[pos] != ord("u") and s[pos] != ord("U")):
            p.append(chr(s[pos]))
            pos += 1
            continue

        p.pop(-1)
        count = 4 if s[pos] == ord("u") else 8
        pos += 1

        # /* \uXXXX with 4 hex digits, \Uxxxxxxxx with 8 */
        number_end = hex_number_end(s, pos, count)
        if number_end - pos != count:
            if not final:
                del p[p_len_before:]
                pos = startinpos
                break
            res = unicode_call_errorhandler(
                errors, "rawunicodeescape", "truncated \\uXXXX", s, pos - 2, number_end
            )
            p.append(res[0])
            pos = res[1]
        else:
            x = int(s[pos : pos + count], 16)
            if x > sys.maxunicode:
                res = unicode_call_errorhandler(
                    errors,
                    "rawunicodeescape",
                    "\\Uxxxxxxxx out of range",
                    s,
                    pos - 2,
                    pos + count,
                )
                pos = res[1]
                p.append(res[0])
            else:
                p.append(chr(x))
                pos += count

    return p, pos
