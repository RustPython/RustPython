import * as rp from '../../lib/pkg';
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
            stdout: '#console'
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

const prompt = ">>>>> ";

const term = new Terminal();
term.open(document.getElementById('terminal'));
term.write(prompt);

function remove_non_ascii(str) {
    if ((str===null) || (str===''))
        return false;
    else
        str = str.toString();

    return str.replace(/[^\x20-\x7E]/g, '');
}

function print_to_console(data) {
    term.write(remove_non_ascii(data) + "\r\n");
}

var input = "";
term.on("data", (data) => {
  const code = data.charCodeAt(0);
  if (code == 13) { // CR
    if (input[input.length - 1] == ':') {
        input += data
        term.write("\r\n.....");
    } else {
        term.write("\r\n");
        try {
            rp.pyEval(input, {
                stdout: print_to_console
            });
        } catch (err) {
            if (err instanceof WebAssembly.RuntimeError) {
                err = window.__RUSTPYTHON_ERROR || err;
            }
            print_to_console(err);
        }
        term.write(prompt);
        input = "";
    }
  } else if (code == 127) {
    term.write("\b \b");
    input = input.slice(0, -1);
  } else if (code < 32 || code == 127) { // Control
    return;
  } else { // Visible
    term.write(data);
    input += data;
  }
});
