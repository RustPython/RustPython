import './style.css';
import CodeMirror from 'codemirror';
import 'codemirror/mode/python/python';
import 'codemirror/addon/comment/comment';
import 'codemirror/lib/codemirror.css';

let rp;

// UI elements
const consoleElement = document.getElementById('console');
const errorElement = document.getElementById('error');
const fetchbtnElement = document.getElementById("fetch-code");
const urlConainerElement = document.getElementById('url-container');

// A dependency graph that contains any wasm must be imported asynchronously.
import('rustpython')
    .then(rustpy => {
        rp = rustpy;
        // so people can play around with it
        window.rp = rustpy;
        onReady();
    })
    .catch(e => {
        console.error('Error importing `rustpython`:', e);
        document.getElementById('error').textContent = e;
    });

// Code Mirror code editor 
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

// Runs the code the the code editor
function runCodeFromTextarea() {
    // Clean the console and errors
    consoleElement.innerHTML = '';
    errorElement.textContent = '';

    const code = editor.getValue();
    try {
        rp.pyExec(code, {
            stdout: output => {
                consoleElement.innerHTML += output;
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

function onReady() {
    // snippets.addEventListener('change', updateSnippet);
    document
        .getElementById('run-btn')
        .addEventListener('click', runCodeFromTextarea);

    // so that the test knows that we're ready
    const readyElement = document.createElement('div');
    readyElement.id = 'rp_loaded';
    document.head.appendChild(readyElement);
}

// when clicking the import code button
// show a UI with a url input + fetch button
// only accepts api.github.com urls (for now)
// add another function to parse a regular url
fetchbtnElement.addEventListener("click", function () {
    // https://developer.github.com/v3/repos/contents/#get-repository-content
    // Format:
    // https://api.github.com/repos/username/reponame/contents/filename.py
    let url = document
        .getElementById('snippet-url')
        .value;
    // minimal js fetch code
    // needs better error handling
    fetch(url)
        .then(res => res.json())
        .then(data => {
            // The Python code is in data.content
            // it is encoded with Base64. Use atob to decode it.
            //https://developer.mozilla.org/en-US/docs/Web/API/WindowOrWorkerGlobalScope/atob
            var decodedData = atob(data.content);
            // set the value of the code editor
            editor.setValue(decodedData);
            urlConainerElement.classList.add("d-none");
        }).catch(err => {
            document
                .getElementById("errors")
                .innerHTML = "Couldn't fetch code. Make sure the link is public."
        });

});

document.getElementById("snippet-btn").addEventListener("click", function () {
    urlConainerElement.classList.remove("d-none");
});