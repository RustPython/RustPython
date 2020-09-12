assert ascii('hello world') == "'hello world'"
assert ascii('안녕 세상') == "'\\uc548\\ub155 \\uc138\\uc0c1'"
assert ascii('안녕 RustPython') == "'\\uc548\\ub155 RustPython'"
assert ascii(5) == '5'
assert ascii(chr(0x10001)) == "'\\U00010001'"
assert ascii(chr(0x9999)) == "'\\u9999'"
assert ascii(chr(0x0A)) == "'\\n'"