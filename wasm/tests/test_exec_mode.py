import time
import sys

from selenium import webdriver
from selenium.webdriver.firefox.options import Options
import pytest

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


def test_eval_mode(driver):
    assert driver.execute_script("return window.rp.pyEval('1+1')") == 2

def test_exec_mode(driver):
    assert driver.execute_script("return window.rp.pyExec('1+1')") is None

def test_exec_single_mode(driver):
    assert driver.execute_script("return window.rp.pyExecSingle('1+1')") == 2
    assert driver.execute_script(
        """
        var output = [];
        save_output = function(text) {{
            output.push(text)
        }};
        window.rp.pyExecSingle('1+1\\n2+2',{stdout: save_output});
        return output;
        """) == ['2\n', '4\n']
