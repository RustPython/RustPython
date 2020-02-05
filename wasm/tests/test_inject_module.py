def test_inject_private(wdriver):
    wdriver.execute_script(
        """
const vm = rp.vmStore.init("vm")
vm.injectModule(
"mod",
`
__all__ = ['get_thing']
def get_thing(): return __thing()
`,
{ __thing: () => 1 },
true
)
vm.execSingle(
`
import mod
assert mod.get_thing() == 1
assert "__thing" not in dir(mod)
try:
    globs = mod.get_thing.__globals__
except TypeError: pass
else:
    assert False, "incognito function.__globals__ didn't error"
`
);
        """
    )

