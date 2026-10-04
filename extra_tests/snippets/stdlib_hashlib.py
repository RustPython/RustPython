import _md5
import _sha1
import hashlib

from testutils import assert_raises

# print(hashlib.md5)
h = hashlib.md5()
h.update(b"a")
g = hashlib.md5(b"a")
assert h.name == g.name == "md5"
print(h.hexdigest())
print(g.hexdigest())

assert h.hexdigest() == g.hexdigest() == "0cc175b9c0f1b6a831c399e269772661"
assert h.digest_size == g.digest_size == 16

h = hashlib.sha256()
h.update(b"a")
g = hashlib.sha256(b"a")
assert h.name == g.name == "sha256"
assert h.digest_size == g.digest_size == 32
print(h.hexdigest())
print(g.hexdigest())

assert (
    h.hexdigest()
    == g.hexdigest()
    == "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb"
)

h = hashlib.sha512()
g = hashlib.sha512(b"a")
assert h.name == g.name == "sha512"
h.update(b"a")
print(h.hexdigest())
print(g.hexdigest())

assert (
    h.hexdigest()
    == g.hexdigest()
    == "1f40fc92da241694750979ee6cf582f2d5d7d28e18335de05abc54d0560e0f5302860c652bf08d560252aa5e74210546f369fbbbce8c12cfc7957b2652fe9a75"
)

h = hashlib.new("blake2s", b"fubar")
print(h.hexdigest())
assert (
    h.hexdigest() == "a0e1ad0c123c9c65e8ef850db2ce4b5cef2c35b06527c615b0154353574d0415"
)
h.update(b"bla")
print(h.hexdigest())
assert (
    h.hexdigest() == "25738bfe4cc104131e1b45bece4dfd4e7e1d6f0dffda1211e996e9d5d3b66e81"
)

# The single-algorithm modules set up their own types rather than relying on
# hashlib having done it.

assert _md5.md5(b"").hexdigest() == "d41d8cd98f00b204e9800998ecf8427e"
assert _sha1.sha1(b"").hexdigest() == "da39a3ee5e6b4b0d3255bfef95601890afd80709"

# a derived key wider than a C int does not fit, and never gets allocated.
# Which OverflowError comes out depends on the width of a C long: where it is
# narrower than the length asked for, converting the argument fails first.
with assert_raises(OverflowError):
    hashlib.pbkdf2_hmac("sha256", b"password", b"salt", 1, 2**62)
