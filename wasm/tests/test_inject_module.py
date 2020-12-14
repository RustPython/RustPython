def test_inject_module_basic(wdriver):
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
`
);
        """
    )
