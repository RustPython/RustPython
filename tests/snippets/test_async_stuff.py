
import sys
import ast

src = """
async def x():
    async for x in [1,2,3]:
        pass
"""

mod = ast.parse(src)
# print(mod)
# print(ast.dump(mod))
