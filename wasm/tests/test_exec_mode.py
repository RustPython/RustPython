def test_eval_mode(wdriver):
    assert wdriver.execute_script("return window.rp.pyEval('1+1')") == 2


def test_exec_mode(wdriver):
    assert wdriver.execute_script("return window.rp.pyExec('1+1')") is None


def test_exec_single_mode(wdriver):
    assert wdriver.execute_script("return window.rp.pyExecSingle('1+1')") == 2
    stdout = wdriver.execute_script(
        """
        let output = "";
        save_output = function(text) {{
            output += text
        }};
        window.rp.pyExecSingle('1+1\\n2+2',{stdout: save_output});
        return output;
        """
    )
    assert stdout == "2\n4\n"


def test_exec_stderr_option(wdriver):
    stderr = wdriver.execute_script(
        """
        let output = "";
        save_output = function(text) {
            output += text
        };
        window.rp.pyExec('import sys; print("err", file=sys.stderr)', {stderr: save_output});
        return output;
        """
    )
    assert stderr == "err\n"
