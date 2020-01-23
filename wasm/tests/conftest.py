import subprocess
import os
import time
import socket
import atexit
import pytest
import sys

PORT = 8080

HTTP_SCRIPT = f"""
import mimetypes
mimetypes.add_type("application/wasm", ".wasm")
import http.server
http.server.test(HandlerClass=http.server.SimpleHTTPRequestHandler, port={PORT})
"""

demo_dist = os.path.join(os.path.dirname(os.path.realpath(__file__)), "../demo/dist/")

server_proc = None


def pytest_sessionstart(session):
    global server_proc
    server_proc = subprocess.Popen(
        ["python3", "-c", HTTP_SCRIPT],
        cwd=demo_dist,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    wait_for_port(PORT)


def pytest_sessionfinish(session):
    global server_proc
    server_proc.terminate()
    server_proc = None


atexit.register(lambda: server_proc and server_proc.terminate())


# From https://gist.github.com/butla/2d9a4c0f35ea47b7452156c96a4e7b12
def wait_for_port(port, host="0.0.0.0", timeout=5.0):
    """Wait until a port starts accepting TCP connections.
    Args:
        port (int): Port number.
        host (str): Host address on which the port should exist.
        timeout (float): In seconds. How long to wait before raising errors.
    Raises:
        TimeoutError: The port isn't accepting connection after time specified in `timeout`.
    """
    start_time = time.perf_counter()
    while True:
        try:
            with socket.create_connection((host, port), timeout=timeout):
                break
        except OSError as ex:
            time.sleep(0.01)
            if time.perf_counter() - start_time >= timeout:
                raise TimeoutError(
                    "Waited too long for the port {} on host {} to start accepting "
                    "connections.".format(port, host)
                ) from ex


from selenium import webdriver
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
from selenium.common.exceptions import JavascriptException


class Driver(webdriver.Firefox):
    def _print_panic(self):
        stack = self.execute_script(
            "return (window.__RUSTPYTHON_ERROR_MSG || '') + '\\n' + (window.__RUSTPYTHON_ERROR_STACK || '')"
        )
        if stack.strip():
            print(f"RustPython panic stack:", stack, file=sys.stderr, sep="\n")

    def execute_script(self, *args, **kwargs):
        try:
            return super().execute_script(*args, **kwargs)
        except JavascriptException:
            self._print_panic()
            raise


@pytest.fixture
def wdriver(request):
    options = Options()
    options.add_argument("-headless")
    driver = Driver(options=options)
    try:
        driver.get(f"http://0.0.0.0:{PORT}")
        WebDriverWait(driver, 5).until(
            EC.presence_of_element_located((By.ID, "rp_loaded"))
        )
    except JavascriptException:
        driver._print_panic()
        driver.quit()
        raise

    yield driver

    driver.quit()
