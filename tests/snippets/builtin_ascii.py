assert ascii('hello world') == "'hello world'"
assert ascii('안녕 세상') == "'\\uc548\\ub155 \\uc138\\uc0c1'"
assert ascii('안녕 RustPython') == "'\\uc548\\ub155 RustPython'"
assert ascii(5) == '5'