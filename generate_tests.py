import os
from pathlib import Path
import re
import sre_constants
import sre_compile
import sre_parse
import json

m = re.search(r"const SRE_MAGIC: usize = (\d+);", open("src/constants.rs").read())
sre_engine_magic = int(m.group(1))
del m

assert sre_constants.MAGIC == sre_engine_magic

class CompiledPattern:
    @classmethod
    def compile(cls, pattern, flags=0):
        p = sre_parse.parse(pattern)
        code = sre_compile._code(p, flags)
        self = cls()
        self.pattern = pattern
        self.code = code
        self.flags = re.RegexFlag(flags | p.state.flags)
        return self

for k, v in re.RegexFlag.__members__.items():
    setattr(CompiledPattern, k, v)

with os.scandir("tests") as d:
    for f in d:
        path = Path(f.path)
        if path.suffix == ".py":
            pattern = eval(path.read_text(), {"re": CompiledPattern})
            path.with_suffix(".re").write_text(
                f"// {pattern.pattern!r}, flags={pattern.flags!r}\n"
                f"Pattern {{ code: &{json.dumps(pattern.code)}, flags: SreFlag::from_bits_truncate({int(pattern.flags)}) }}"
            )
