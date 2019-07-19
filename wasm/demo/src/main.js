import * as rp from 'rustpython';
import CodeMirror from 'codemirror';
import 'codemirror/mode/python/python';
import 'codemirror/addon/comment/comment';
import { Terminal } from 'xterm';

// so people can play around with it
window.rp = rp;

const editor = CodeMirror.fromTextArea(document.getElementById('code'), {
    extraKeys: {
        'Ctrl-Enter': runCodeFromTextarea,
        'Cmd-Enter': runCodeFromTextarea,
        'Shift-Tab': 'indentLess',
        'Ctrl-/': 'toggleComment',
        'Cmd-/': 'toggleComment',
        Tab: editor => {
            var spaces = Array(editor.getOption('indentUnit') + 1).join(' ');
            editor.replaceSelection(spaces);
        }
    },
    lineNumbers: true,
    mode: 'text/x-python',
    indentUnit: 4,
    autofocus: true
});

const consoleElement = document.getElementById('console');
const errorElement = document.getElementById('error');

function runCodeFromTextarea() {
    // Clean the console and errors
    consoleElement.value = '';
    errorElement.textContent = '';

    const code = editor.getValue();
    try {
        const result = rp.pyEval(code, {
            stdout: output => {
                const shouldScroll =
                    consoleElement.scrollHeight - consoleElement.scrollTop ===
                    consoleElement.clientHeight;
                consoleElement.value += output;
                if (shouldScroll) {
                    consoleElement.scrollTop = consoleElement.scrollHeight;
                }
            }
        });
        if (result !== null) {
            consoleElement.value += `\n${result}\n`;
        }
    } catch (err) {
        if (err instanceof WebAssembly.RuntimeError) {
            err = window.__RUSTPYTHON_ERROR || err;
        }
        errorElement.textContent = err;
        console.error(err);
    }
}

document
    .getElementById('run-btn')
    .addEventListener('click', runCodeFromTextarea);

const snippets = document.getElementById('snippets');

const updateSnippet = () => {
    const selected = snippets.value;

    // the require here creates a webpack context; it's fine to use it
    // dynamically.
    // https://webpack.js.org/guides/dependency-management/
    const snippet = require(`raw-loader!../snippets/${selected}.py`);

    editor.setValue(snippet);
    runCodeFromTextarea();
};

snippets.addEventListener('change', updateSnippet);

// Run once for demo (updateSnippet b/c the browser might try to keep the same
// option selected for the `select`, but the textarea won't be updated)
updateSnippet();

function removeNonAscii(str) {
    if (str === null || str === '') return false;
    else str = str.toString();

    return str.replace(/[^\x20-\x7E]/g, '');
}

function printToConsole(data) {
    term.write(removeNonAscii(data) + '\r\n');
}

const term = new Terminal();
term.open(document.getElementById('terminal'));

const terminalVM = rp.vmStore.init('term_vm');
terminalVM.setStdout(printToConsole);

function getPrompt(name = 'ps1') {
    terminalVM.exec(`
try:
    import sys as __sys
    __prompt = __sys.${name}
except:
    __prompt = ''
finally:
    del __sys
`);
    return String(terminalVM.eval('__prompt'));
}

term.write(getPrompt());

function resetInput() {
    continuedInput = [];
    input = '';
    continuing = false;
}

let continuedInput, input, continuing;
resetInput();

let ps2;

term.on('data', data => {
    const code = data.charCodeAt(0);
    if (code == 13) {
        // CR
        term.write('\r\n');
        continuedInput.push(input);
        if (continuing) {
            if (input === '') {
                continuing = false;
            } else {
                input = '';
                term.write(ps2);
                return;
            }
        }
        try {
            terminalVM.execSingle(continuedInput.join('\n'));
        } catch (err) {
            if (err instanceof SyntaxError && err.message.includes('EOF')) {
                ps2 = getPrompt('ps2');
                term.write(ps2);
                continuing = true;
                input = '';
                return;
            } else if (err instanceof WebAssembly.RuntimeError) {
                err = window.__RUSTPYTHON_ERROR || err;
            }
            printToConsole(err);
        }
        resetInput();
        term.write(getPrompt());
    } else if (code == 127 || code == 8) {
        // Backspace
        if (input.length > 0) {
            term.write('\b \b');
            input = input.slice(0, -1);
        }
    } else if (code < 32) {
        // Control
        term.write('\r\n' + getPrompt());
        input = '';
        continuedInput = [];
    } else {
        // Visible
        term.write(data);
        input += data;
    }
});
