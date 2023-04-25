import os
import argparse
import shutil
import subprocess

def main():
    parser = argparse.ArgumentParser(description="Test cpython library")
    parser.add_argument("-cp", nargs=1, default="../../../cpython", required=False, help="Local cpython path.")
    parser.add_argument("-rp", nargs=1, default="../..", required=False, help="Local RustPython path.")
    args = vars(parser.parse_args())
    CPYTHONPATH = args["cp"]
    RPYTHONPATH = args["rp"]

    if isinstance(CPYTHONPATH, list):
        CPYTHONPATH = CPYTHONPATH[0]
    if isinstance(RPYTHONPATH, list):
        RPYTHONPATH = RPYTHONPATH[0]

    liblst = list()

    with open("clib_list.txt", 'r') as f:
        for line in f.readlines():
            line = line[:line.find('#')]
            line = line.strip()
            if line:
                liblst.append(line)

    for lib in liblst:
        lib_exist = False
        test_exist = False
        test_do = True
        message = [f"{lib}:"]

        if os.path.isfile(f"{RPYTHONPATH}/Lib/{lib}.py"):
            lib_exist = True
            os.rename(f"{RPYTHONPATH}/Lib/{lib}.py", f"{RPYTHONPATH}/Lib/{lib}_tmp_mv.py")
        if os.path.isfile(f"{RPYTHONPATH}/Lib/test/test_{lib}.py"):
            test_exist = True
            os.rename(f"{RPYTHONPATH}/Lib/test/test_{lib}.py", f"{RPYTHONPATH}/Lib/test/test_{lib}_tmp_mv.py")

        if os.path.isfile(f"{CPYTHONPATH}/Lib/{lib}.py"):
            shutil.copyfile(f"{CPYTHONPATH}/Lib/{lib}.py", f"{RPYTHONPATH}/Lib/{lib}.py")
        else:
            test_do = False
            message.append(f"No cpython/Lib/{lib}.py")

        if os.path.isfile(f"{CPYTHONPATH}/Lib/test/test_{lib}.py"):
            shutil.copyfile(f"{CPYTHONPATH}/Lib/test/test_{lib}.py", f"{RPYTHONPATH}/Lib/test/test_{lib}.py")
        else:
            test_do = False
            message.append(f"No cpython/Lib/test/test_{lib}.py")
        
        if test_do:
            result = subprocess.run(["cargo", "run", "-q", f"{RPYTHONPATH}/Lib/test/test_{lib}.py"], stdout=subprocess.PIPE)
            result = result.stdout.decode("utf-8")

            if "OK" in result:
                message.append("OK")
            else:
                message.append("Failed")
        
        if lib_exist:
            os.rename(f"{RPYTHONPATH}/Lib/{lib}_tmp_mv.py", f"{RPYTHONPATH}/Lib/{lib}.py")
        elif os.path.isfile(f"{RPYTHONPATH}/Lib/{lib}.py"):
            os.remove(f"{RPYTHONPATH}/Lib/{lib}.py")

        if test_exist:
            os.rename(f"{RPYTHONPATH}/Lib/test/test_{lib}_tmp_mv.py", f"{RPYTHONPATH}/Lib/test/test_{lib}.py")
        elif os.path.isfile(f"{RPYTHONPATH}/Lib/test/test_{lib}.py"):
            os.remove(f"{RPYTHONPATH}/Lib/test/test_{lib}.py")

        message.append('\n')
        message = '  '.join(message)

        with open("clib_out.txt", "a") as f:
            f.write(message)

if __name__ == "__main__":
    main()