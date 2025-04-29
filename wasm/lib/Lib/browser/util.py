from _browser import window # type: ignore

jsstr = window.new_from_str
jsclosure = window.new_closure
jsclosure_once = window.new_closure_once

def jsfloat(n):
    return window.new_from_float(float(n))

UNDEFINED = window.undefined()
NULL = window.null()
