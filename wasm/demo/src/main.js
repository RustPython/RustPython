import * as rp from 'rustpython';
import CodeMirror from 'codemirror';
import 'codemirror/mode/python/python';
import 'codemirror/addon/comment/comment';
import { Terminal } from 'xterm';
import LocalEchoController from 'local-echo';

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
        rp.pyEval(code, {
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

const term = new Terminal();
term.open(document.getElementById('terminal'));

const localEcho = new LocalEchoController(term);

const terminalVM = rp.vmStore.init('term_vm');

terminalVM.setStdout(data => localEcho.println(data));

function getPrompt(name) {
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

async function readPrompts() {
    let continuing = false;

    while (true) {
        const ps1 = getPrompt('ps1');
        const ps2 = getPrompt('ps2');
        let input;
        if (continuing) {
            const prom = localEcho.read(ps2, ps2);
            localEcho._activePrompt.prompt = ps1;
            localEcho._input = localEcho.history.entries.pop() + '\n';
            localEcho._cursor = localEcho._input.length;
            localEcho._active = true;
            input = await prom;
            if (!input.endsWith('\n')) continue;
        } else {
            input = await localEcho.read(ps1, ps2);
        }
        try {
            terminalVM.execSingle(input);
        } catch (err) {
            if (err instanceof SyntaxError && err.message.includes('EOF')) {
                continuing = true;
                continue;
            } else if (err instanceof WebAssembly.RuntimeError) {
                err = window.__RUSTPYTHON_ERROR || err;
            }
            localEcho.println(err);
        }
        continuing = false;
    }
}

readPrompts().catch(err => console.error(err));
