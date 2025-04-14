import argparse

def parse_args():
    parser = argparse.ArgumentParser(description="Fix test.")
    parser.add_argument("--test", type=str, help="Name of test")
    parser.add_argument("--path", type=str, help="Path to test file")
    parser.add_argument("--force", action="store_true", help="Force modification")

    args = parser.parse_args()
    return args

class Test:
    name: str = ""
    path: str = ""
    result: str = ""

    def __str__(self):
        return f"Test(name={self.name}, path={self.path}, result={self.result})"

class TestResult:
    tests_result: str = ""
    tests = []

    def __str__(self):
        return f"TestResult(tests_result={self.tests_result},tests={len(self.tests)})"


def parse_results(result):
    lines = result.stdout.splitlines()
    test_results = TestResult()
    in_test_results = False
    for line in lines:
        if line == "Run tests sequentially":
            in_test_results = True
        elif line.startswith("-----------"):
            in_test_results = False
        if in_test_results and not line.startswith("tests") and not line.startswith("["):
            line = line.split(" ")
            if line != [] and len(line) > 3:
                test = Test()
                test.name = line[0]
                test.path = line[1].strip("(").strip(")")
                test.result = " ".join(line[3:]).lower()
                test_results.tests.append(test)
        else:
            if "== Tests result: " in line:
                res = line.split("== Tests result: ")[1]
                res = res.split(" ")[0]
                test_results.tests_result = res
    return test_results

def path_to_test(path) -> list[str]:
    return path.split(".")[2:]

def modify_test(file: str, test: list[str]) -> str:
    lines = file.splitlines()
    result = []
    for line in lines:
        if line.lstrip(" ").startswith("def " + test[-1]):
            whitespace = line[:line.index("def ")]
            result.append(whitespace + "# TODO: RUSTPYTHON")
            result.append(whitespace + "@unittest.expectedFailure")
        result.append(line)
    return "\n".join(result)

def run_test(test_name):
    print(f"Running test: {test_name}")
    rustpython_location = "./target/release/rustpython"
    import subprocess
    result = subprocess.run([rustpython_location, "-m", "test", "-v", test_name], capture_output=True, text=True)
    return parse_results(result)


if __name__ == "__main__":
    args = parse_args()
    test_name = args.test
    tests = run_test(test_name)
    f = open(args.path).read()
    for test in tests.tests:
        if test.result == "fail":
            print("Modifying test:", test.name)
            f = modify_test(f, path_to_test(test.path))
    with open(args.path, "w") as file:
        if args.force or run_test().tests_result == "ok":
            file.write(f)
        else:
            raise Exception("Test failed after modification")
