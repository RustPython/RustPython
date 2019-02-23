import * as rp from '../../lib/pkg';
import CodeMirror from 'codemirror';
import 'codemirror/mode/python/python';
import 'codemirror/addon/comment/comment';

// so people can play around with it
window.rp = rp;

const editor = CodeMirror.fromTextArea(document.getElementById('code'), {
    extraKeys: {
        'Ctrl-Enter': runCodeFromTextarea,
        'Cmd-Enter': runCodeFromTextarea,
        'Shift-Tab': 'indentLess',
        'Ctrl-/': 'toggleComment',
        'Cmd-/': 'toggleComment'
    },
    lineNumbers: true,
    mode: 'text/x-python',
    indentUnit: 4,
    autofocus: true
});

function runCodeFromTextarea() {
    const consoleElement = document.getElementById('console');
    const errorElement = document.getElementById('error');

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
    } catch (e) {
        errorElement.textContent = e;
        console.error(e);
    }
}

document
    .getElementById('run-btn')
    .addEventListener('click', runCodeFromTextarea);

const snippets = document.getElementById('snippets');

snippets.addEventListener('change', () => {
    const selected = snippets.value;

    // the require here creates a webpack context; it's fine to use it
    // dynamically.
    // https://webpack.js.org/guides/dependency-management/
    const snippet = require(`raw-loader!./snippets/${selected}.py`);

    editor.setValue(snippet);

    runCodeFromTextarea();
});

runCodeFromTextarea(); // Run once for demo
