import './style.css';
import '@xterm/xterm/css/xterm.css';
import { EditorView, basicSetup } from 'codemirror';
import { keymap } from '@codemirror/view';
import { indentUnit } from '@codemirror/language';
import { indentWithTab } from '@codemirror/commands';
import { python } from '@codemirror/lang-python';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { Readline } from 'xterm-readline';

let rp;

// A dependency graph that contains any wasm must be imported asynchronously.
import('rustpython')
    .then((rustpython) => {
        rp = rustpython;
        // so people can play around with it
        window.rp = rustpython;
        onReady();
    })
    .catch((e) => {
        console.error('Error importing `rustpython`:', e);
        let errorDetails = e.toString();
        if (window.__RUSTPYTHON_ERROR) {
            errorDetails += '\nRustPython Error: ' + window.__RUSTPYTHON_ERROR;
        }
        if (window.__RUSTPYTHON_ERROR_STACK) {
            errorDetails += '\nStack: ' + window.__RUSTPYTHON_ERROR_STACK;
        }
        document.getElementById('error').textContent = errorDetails;
    });

const fixedHeightEditor = EditorView.theme({
    '&': { height: '100%' },
    '.cm-scroller': { overflow: 'auto' },
});
const editor = new EditorView({
    parent: document.getElementById('code-wrapper'),
    extensions: [
        basicSetup,
        python(),
        keymap.of(
            { key: 'Ctrl-Enter', mac: 'Cmd-Enter', run: runCodeFromTextarea },
            indentWithTab,
        ),
        indentUnit.of('    '),
        fixedHeightEditor,
    ],
});
editor.focus();

const consoleElement = document.getElementById('console');
const errorElement = document.getElementById('error');

function runCodeFromTextarea() {
    // Clean the console and errors
    consoleElement.value = '';
    errorElement.textContent = '';

    const code = editor.state.doc.toString();
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
    const snippet = require(`../snippets/${selected}.py?raw`);

    editor.dispatch({
        changes: { from: 0, to: editor.state.doc.length, insert: snippet },
    });
}
function updateSnippetAndRun() {
    updateSnippet();
    requestAnimationFrame(runCodeFromTextarea);
}
updateSnippet();

const term = new Terminal();
const readline = new Readline();
const fitAddon = new FitAddon();
term.loadAddon(readline);
term.loadAddon(fitAddon);
term.open(document.getElementById('terminal'));
fitAddon.fit();

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
    let continuing = '';

    while (true) {
        let input = await readline.read(getPrompt(continuing ? 'ps2' : 'ps1'));
        if (input.endsWith('\n')) input = input.slice(0, -1);
        if (continuing) {
            input = continuing += '\n' + input;
            if (!continuing.endsWith('\n')) continue;
        }
        try {
            console.log([input]);
            terminalVM.execSingle(input);
        } catch (err) {
            if (err.canContinue) {
                continuing = input;
                continue;
            } else if (err instanceof WebAssembly.RuntimeError) {
                err = window.__RUSTPYTHON_ERROR || err;
            }
            readline.print('' + err);
        }
        continuing = '';
    }
}

function onReady() {
    snippets.addEventListener('change', updateSnippetAndRun);
    document
        .getElementById('run-btn')
        .addEventListener('click', runCodeFromTextarea);
    // Run once for demo
    runCodeFromTextarea();

    terminalVM = rp.vmStore.init('term_vm');
    terminalVM.setStdout((data) => readline.print(data));
    readPrompts().catch((err) => console.error(err));

    // so that the test knows that we're ready
    const readyElement = document.createElement('div');
    readyElement.id = 'rp_loaded';
    document.head.appendChild(readyElement);
}
