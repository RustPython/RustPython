from testutils import assert_raises

#test only makes sense with python 3.8 or higher (or RustPython)
import sys
import platform
if platform.python_implementation() == 'CPython':
    assert sys.version_info >= (3, 8), 'Incompatible Python Version, expected CPython 3.8 or later'
elif platform.python_implementation == 'RustPython':
    # ok
    pass
else:
    # other implementation - lets give it a try
    pass


# lets start tersing
foo = 'bar'

assert f"{''}" == ''
assert f"{f'{foo}'}" == 'bar'
assert f"foo{foo}" == 'foobar'
assert f"{foo}foo" == 'barfoo'
assert f"foo{foo}foo" == 'foobarfoo'
assert f"{{foo}}" == '{foo}'
assert f"{ {foo} }" == "{'bar'}"
assert f"{f'{{}}'}" == '{}' # don't include escaped braces in nested f-strings
assert f'{f"{{"}' == '{'
assert f'{f"}}"}' == '}'
assert f'{foo}' f"{foo}" 'foo' == 'barbarfoo'
assert f'{"!:"}' == '!:'
assert fr'x={4*10}\n' == 'x=40\\n'
assert f'{16:0>+#10x}' == '00000+0x10'
assert f"{{{(lambda x: f'hello, {x}')('world}')}" == '{hello, world}'


# base test of self documenting strings
#assert f'{foo=}' == 'foo=bar' # TODO ' missing

num=42

f'{num=}' # keep this line as it will fail when using a python 3.7 interpreter 

assert f'{num=}' == 'num=42'
assert f'{num=:>10}' == 'num=        42'


spec = "0>+#10x"
assert f"{16:{spec}}{foo}" == '00000+0x10bar'

# TODO:
# spec = "bla"
# assert_raises(ValueError, lambda: f"{16:{spec}}")

# Normally `!` cannot appear outside of delimiters in the expression but
# cpython makes an exception for `!=`, so we should too.

# assert f'{1 != 2}' == 'True'


# conversion flags

class Value:
    def __format__(self, spec):
        return "foo"

    def __repr__(self):
        return "bar"

    def __str__(self):
        return "baz"

v = Value()

assert f'{v}' == 'foo'
assert f'{v!r}' == 'bar'
assert f'{v!s}' == 'baz'
assert f'{v!a}' == 'bar'

# advanced expressions:

assert f'{True or True}' == 'True'
assert f'{1 == 1}' == 'True'
assert f'{"0" if True else "1"}' == '0'

# Test ascii representation of unicodes:
v = "\u262e"
assert f'>{v}' == '>\u262e'
assert f'>{v!r}' == ">'\u262e'"
assert f'>{v!s}' == '>\u262e'
assert f'>{v!a}' == r">'\u262e'"



# Test format specifier after conversion flag
#assert f'{"42"!s:<5}' == '42   ', '#' + f'{"42"!s:5}' +'#' # TODO: default alignment in cpython is left

assert f'{"42"!s:<5}' == '42   ', '#' + f'{"42"!s:<5}' +'#'
assert f'{"42"!s:>5}' == '   42', '#' + f'{"42"!s:>5}' +'#'

#assert f'{"42"=!s:5}' == '"42"=42   ', '#'+ f'{"42"=!s:5}' +'#' # TODO default alingment in cpython is left
assert f'{"42"=!s:<5}' == '"42"=42   ', '#'+ f'{"42"=!s:<5}' +'#'
assert f'{"42"=!s:>5}' == '"42"=   42', '#'+ f'{"42"=!s:>5}' +'#'



### Tests for fstring selfdocumenting form CPython

class C:
    def assertEqual(self, a,b):
        assert a==b, "{0} == {1}".format(a,b)

self=C()

x = 'A string'
self.assertEqual(f'{10=}', '10=10')
# self.assertEqual(f'{x=}', 'x=' + x )#repr(x)) # TODO: add  ' when printing strings
# self.assertEqual(f'{x =}', 'x =' + x )# + repr(x)) # TODO: implement '  handling
self.assertEqual(f'{x=!s}', 'x=' + str(x))
# # self.assertEqual(f'{x=!r}', 'x=' + x) #repr(x)) # !r not supported
# self.assertEqual(f'{x=!a}', 'x=' + ascii(x))

x = 2.71828
self.assertEqual(f'{x=:.2f}', 'x=' + format(x, '.2f'))
self.assertEqual(f'{x=:}', 'x=' + format(x, ''))
self.assertEqual(f'{x=!r:^20}', 'x=' + format(repr(x), '^20')) # TODO formatspecifier after conversion flsg is currently not supported (also for classical fstrings)
self.assertEqual(f'{x=!s:^20}', 'x=' + format(str(x), '^20'))
self.assertEqual(f'{x=!a:^20}', 'x=' + format(ascii(x), '^20'))

x = 9
self.assertEqual(f'{3*x+15=}', '3*x+15=42')

# There is code in ast.c that deals with non-ascii expression values.  So,
# use a unicode identifier to trigger that.
tenπ = 31.4
self.assertEqual(f'{tenπ=:.2f}', 'tenπ=31.40')

# Also test with Unicode in non-identifiers.
#self.assertEqual(f'{"Σ"=}', '"Σ"=\'Σ\'') ' TODO ' missing

# Make sure nested fstrings still work.
self.assertEqual(f'{f"{3.1415=:.1f}":*^20}', '*****3.1415=3.1*****')

# Make sure text before and after an expression with = works
# correctly.
pi = 'π'
#self.assertEqual(f'alpha α {pi=} ω omega', "alpha α pi='π' ω omega") # ' missing around pi

# Check multi-line expressions.
#self.assertEqual(f'''{3=}''', '\n3\n=3') # TODO: multiline f strings not supported, seems to be an rustpython issue

# Since = is handled specially, make sure all existing uses of
# it still work.

self.assertEqual(f'{0==1}', 'False')
self.assertEqual(f'{0!=1}', 'True')
self.assertEqual(f'{0<=1}', 'True')
self.assertEqual(f'{0>=1}', 'False')

# Make sure leading and following text works.
# x = 'foo'
#self.assertEqual(f'X{x=}Y', 'Xx='+repr(x)+'Y') # TODO ' 
# self.assertEqual(f'X{x=}Y', 'Xx='+x+'Y') # just for the moment

# Make sure whitespace around the = works.
# self.assertEqual(f'X{x  =}Y', 'Xx  ='+repr(x)+'Y')  # TODO '
# self.assertEqual(f'X{x=  }Y', 'Xx=  '+repr(x)+'Y') # TODO '
# self.assertEqual(f'X{x  =  }Y', 'Xx  =  '+repr(x)+'Y') # TODO '

# self.assertEqual(f'X{x  =}Y', 'Xx  ='+x+'Y')
# self.assertEqual(f'X{x=  }Y', 'Xx=  '+x+'Y')
# self.assertEqual(f'X{x  =  }Y', 'Xx  =  '+x+'Y')
