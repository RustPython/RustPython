import time
import sys

import pytest

RUN_CODE_TEMPLATE = """
var output = "";
save_output = function(text) {{
	output += text
}};
var vm = window.rp.vmStore.init('test_vm');
vm.setStdout(save_output);
vm.exec('{}');
vm.destroy();
return output;
"""


@pytest.mark.parametrize(
    "script, output", [("print(5)", "5"), ("a=5;b=4;print(a+b)", "9")]
)
def test_demo(wdriver, script, output):
    script = RUN_CODE_TEMPLATE.format(script)
    script_output = wdriver.execute_script(script)
    assert script_output.strip() == output
