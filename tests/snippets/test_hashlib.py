
import hashlib

# print(hashlib.md5)
h = hashlib.md5()
h.update(b'a')
print(h.hexdigest())

assert h.hexdigest() == '0cc175b9c0f1b6a831c399e269772661'

h = hashlib.sha256()
h.update(b'a')
print(h.hexdigest())

assert h.hexdigest() == 'ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb'

h = hashlib.sha512()
h.update(b'a')
print(h.hexdigest())

assert h.hexdigest() == '1f40fc92da241694750979ee6cf582f2d5d7d28e18335de05abc54d0560e0f5302860c652bf08d560252aa5e74210546f369fbbbce8c12cfc7957b2652fe9a75'
