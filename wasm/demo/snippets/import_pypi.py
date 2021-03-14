import asyncweb
import whlimport

whlimport.setup()

# make sys.modules['os'] a dumb version of the os module, which has posixpath
# available as os.path as well as a few other utilities, but will raise an
# OSError for anything that actually requires an OS
import _dummy_os
_dummy_os._shim()

@asyncweb.main
async def main():
    await whlimport.load_package("pygments")
    import pygments
    import pygments.lexers
    import pygments.formatters.html
    lexer = pygments.lexers.get_lexer_by_name("python")
    fmter = pygments.formatters.html.HtmlFormatter(noclasses=True, style="default")
    print(pygments.highlight("print('hi, mom!')", lexer, fmter))
