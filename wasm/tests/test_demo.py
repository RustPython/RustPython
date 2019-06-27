import time
import sys

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

def print_stack(driver):
	stack = driver.execute_script(
		"return window.__RUSTPYTHON_ERROR_MSG + '\\n' + window.__RUSTPYTHON_ERROR_STACK"
	)
	print(f"RustPython error stack:\n{stack}", file=sys.stderr)


@pytest.fixture(scope="module")
def driver(request):
	options = Options()
	options.add_argument('-headless')
	driver = webdriver.Firefox(options=options)
	try:
		driver.get("http://localhost:8080")
	except Exception as e:
		print_stack(driver)
		raise
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
	try:
		script_output = driver.execute_script(script)
	except Exception as e:
		print_stack(driver)
		raise
	assert script_output.strip() == output
