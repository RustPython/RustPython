import './style.css';
import 'xterm/lib/xterm.css';
import CodeMirror from 'codemirror';
import 'codemirror/mode/python/python';
import 'codemirror/addon/comment/comment';
import 'codemirror/lib/codemirror.css';
import { Terminal } from 'xterm';
import LocalEchoController from 'local-echo';

let rp;

// A dependency graph that contains any wasm must be imported asynchronously.
import('rustpython')
    .then((rustpy) => {
        rp = rustpy;
        // so people can play around with it
        window.rp = rustpy;
        onReady();
    })
    .catch((e) => {
        console.error('Error importing `rustpython`:', e);
        document.getElementById('error').textContent = e;
    });

const editor = CodeMirror.fromTextArea(document.getElementById('code'), {
    extraKeys: {
        'Ctrl-Enter': runCodeFromTextarea,
        'Cmd-Enter': runCodeFromTextarea,
        'Shift-Tab': 'indentLess',
        'Ctrl-/': 'toggleComment',
        'Cmd-/': 'toggleComment',
        Tab: (editor) => {
            var spaces = Array(editor.getOption('indentUnit') + 1).join(' ');
            editor.replaceSelection(spaces);
        },
    },
    lineNumbers: true,
    mode: 'text/x-python',
    indentUnit: 4,
    autofocus: true,
});

const consoleElement = document.getElementById('console');
const errorElement = document.getElementById('error');

function runCodeFromTextarea() {
    // Clean the console and errors
    consoleElement.value = '';
    errorElement.textContent = '';

    const code = editor.getValue();
    try {
        rp.pyExec(code, {
            stdout: (output) => {
                const shouldScroll =
                    consoleElement.scrollHeight - consoleElement.scrollTop ===
                    consoleElement.clientHeight;
                consoleElement.value += output;
                if (shouldScroll) {
                    consoleElement.scrollTop = consoleElement.scrollHeight;
                }
            },
        });
    } catch (err) {
        if (err instanceof WebAssembly.RuntimeError) {
            err = window.__RUSTPYTHON_ERROR || err;
        }
        errorElement.textContent = err;
        console.error(err);
    }
}

const snippets = document.getElementById('snippets');

function updateSnippet() {
    const selected = snippets.value;

    // the require here creates a webpack context; it's fine to use it
    // dynamically.
    // https://webpack.js.org/guides/dependency-management/
    const {
        default: snippet,
    } = require(`raw-loader!../snippets/${selected}.py`);

    editor.setValue(snippet);
    runCodeFromTextarea();
}

const term = new Terminal();
term.open(document.getElementById('terminal'));

const localEcho = new LocalEchoController(term);

let terminalVM;

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
            if (err.canContinue) {
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

function onReady() {
    snippets.addEventListener('change', updateSnippet);
    document
        .getElementById('run-btn')
        .addEventListener('click', runCodeFromTextarea);
    // Run once for demo
    runCodeFromTextarea();

    terminalVM = rp.vmStore.init('term_vm');
    terminalVM.setStdout((data) => localEcho.print(data));
    readPrompts().catch((err) => console.error(err));

    // so that the test knows that we're ready
    const readyElement = document.createElement('div');
    readyElement.id = 'rp_loaded';
    document.head.appendChild(readyElement);
}
