import time

from selenium import webdriver
from selenium.webdriver.firefox.options import Options
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

@pytest.fixture(scope="module")
def driver(request):
	options = Options()
	options.add_argument('-headless')
	driver = webdriver.Firefox(options=options)
	driver.get("http://localhost:8080")
	time.sleep(5)
	yield driver
	driver.close()


@pytest.mark.parametrize("script, output",
	[
		("print(5)", "5"),
		("a=5;b=4;print(a+b)", "9")
	]
)
def test_demo(driver, script, output):
	script = RUN_CODE_TEMPLATE.format(script)
	assert driver.execute_script(script).strip() == output
