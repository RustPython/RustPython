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
    document
        .getElementById('run-btn')
        .addEventListener('click', runCodeFromTextarea);

    // so that the test knows that we're ready
    const readyElement = document.createElement('div');
    readyElement.id = 'rp_loaded';
    document.head.appendChild(readyElement);
}

// import button
// show a url input + fetch button
// takes a url where there is raw code
fetchbtnElement.addEventListener("click", function () {
    let url = document
        .getElementById('snippet-url')
        .value;
    // minimal js fetch code
    // needs better error handling
    fetch(url)
        .then( response => {
            if (!response.ok) { throw response }
            return response.text() 
        })
        .then(text => {
            // set the value of the code editor
            editor.setValue(text);
            // hide the ui
            urlConainerElement.classList.add("d-none");
        }).catch(err => {
            // show the error as is for troubleshooting.
            document
                .getElementById("error")
                .innerHTML = err
        });

});

document.getElementById("snippet-btn").addEventListener("click", function () {
    urlConainerElement.classList.remove("d-none");
});