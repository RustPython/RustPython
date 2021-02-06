import asyncweb
import pypimport

pypimport.setup()

# shim path utilities into the "os" module
class os:
    import posixpath as path
import sys
sys.modules['os'] = os
sys.modules['os.path'] = os.path
del sys, os

@asyncweb.main
async def main():
    await pypimport.load_package("pygments")
    import pygments
    import pygments.lexers
    import pygments.formatters.html
    lexer = pygments.lexers.get_lexer_by_name("python")
    fmter = pygments.formatters.html.HtmlFormatter(noclasses=True, style="default")
    print(pygments.highlight("print('hi, mom!')", lexer, fmter))
