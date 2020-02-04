def test_inject_private(wdriver):
    assert wdriver.execute_script(
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
return vm.execSingle(
    `import mod; mod.get_thing() == 1 and "__thing" not in dir(mod)`
);
        """
    )

