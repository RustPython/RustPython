
import sys
import ast

src = """
async def x():
    async for x in [1,2,3]:
        await y()
"""

mod = ast.parse(src)
# print(mod)
# print(ast.dump(mod))
