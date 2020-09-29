import './style.css';
// Code Mirror (https://codemirror.net/)
// https://github.com/codemirror/codemirror
import CodeMirror from 'codemirror';
import 'codemirror/mode/python/python';
import 'codemirror/mode/markdown/markdown';
import 'codemirror/mode/stex/stex';
import 'codemirror/addon/comment/comment';
import 'codemirror/lib/codemirror.css';

// MarkedJs (https://marked.js.org/)
// Renders Markdown
// https://github.com/markedjs/marked
import marked from 'marked';

// KaTex (https://katex.org/)
// Renders Math
// https://github.com/KaTeX/KaTeX
import katex from 'katex';
import 'katex/dist/katex.min.css';

// Parses the code and splits it to chunks
// uses %% keyword for separators
// copied from iodide project
// https://github.com/iodide-project/iodide/blob/master/src/editor/iomd-tools/iomd-parser.js
import { iomdParser } from './parser';

let rp;

const notebook = document.getElementById('rp-notebook');
const error = document.getElementById('error');

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

// Code Editor
const editor = CodeMirror.fromTextArea(document.getElementById('code'), {
    extraKeys: {
        'Ctrl-Enter': parseCodeFromEditor,
        'Cmd-Enter': parseCodeFromEditor,
        'Shift-Tab': 'indentLess',
        'Ctrl-/': 'toggleComment',
        'Cmd-/': 'toggleComment',
        Tab: (editor) => {
            var spaces = Array(editor.getOption('indentUnit') + 1).join(' ');
            editor.replaceSelection(spaces);
        },
    },
    lineNumbers: true,
    mode: 'text/x-notebook',
    indentUnit: 4,
    autofocus: true,
    lineWrapping: true,
});

// Parses what is the code editor
// either runs python or renders math or markdown
function parseCodeFromEditor() {
    // Clean the console and errors
    notebook.innerHTML = '';
    error.textContent = '';

    // gets the code from code editor
    let code = editor.getValue();

    /* 
    Split code into chunks.
    Uses %%keyword or %% keyword as separator
    Implemented %%py %%md %%math for python, markdown and math.
    Returned object has: 
        - chunkContent, chunkType, chunkId, 
        - evalFlags, startLine, endLine 
    */
    let parsed_code = iomdParser(code);

    parsed_code.forEach((chunk) => {
        // For each type of chunk, do somthing
        // so far have py for python, md for markdown and math for math ;p
        let content = chunk.chunkContent;
        switch (chunk.chunkType) {
            // by default assume this is python code
            // so users don't have to type py manually
            case '':
            case 'py':
                runPython(content);
                break;
            case 'md':
                notebook.innerHTML += renderMarkdown(content);
                break;
            case 'math':
                notebook.innerHTML += renderMath(content, true);
                break;
            case 'math-inline':
                notebook.innerHTML += renderMath(content, false);
                break;
            default:
            // do nothing when we see an unknown chunk for now
        }
    });
}

// Run Python code
function runPython(code) {
    try {
        rp.pyExec(code, {
            stdout: (output) => {
                notebook.innerHTML += output;
            },
        });
    } catch (err) {
        if (err instanceof WebAssembly.RuntimeError) {
            err = window.__RUSTPYTHON_ERROR || err;
        }
        error.textContent = err;
    }
}

// Render Markdown with imported marked compiler
function renderMarkdown(md) {
    // TODO: add error handling and output sanitization
    let settings = {
        headerIds: true,
        breaks: true,
    };

    return marked(md, settings);
}

// Render Math with Katex
function renderMath(math, display_mode) {
    // TODO: definetly add error handling.
    return katex.renderToString(math, {
        displayMode: display_mode,
        macros: { '\\f': '#1f(#2)' },
    });
}

function onReady() {
    /* By default the notebook has the keyword "loading"
    once python and doc is ready:
    create an empty div and set the id to 'rp_loaded'
    so that the test knows that we're ready */
    const readyElement = document.createElement('div');
    readyElement.id = 'rp_loaded';
    document.head.appendChild(readyElement);
    // set the notebook to empty
    notebook.innerHTML = '';
}

// on click, parse the code
document
    .getElementById('run-btn')
    .addEventListener('click', parseCodeFromEditor);

// import button
// show a url input + fetch button
// takes a url where there is raw code
document.getElementById('fetch-code').addEventListener('click', function () {
    let url = document.getElementById('snippet-url').value;
    // minimal js fetch code
    // TODO: better error handling
    fetch(url)
        .then((response) => {
            if (!response.ok) {
                throw response;
            }
            return response.text();
        })
        .then((text) => {
            // set the value of the code editor
            editor.setValue(text);
            // hide the ui
            document.getElementById('url-container').classList.add('d-none');
        })
        .catch((err) => {
            // show the error as is for troubleshooting.
            document.getElementById('error').innerHTML = err;
        });
});

// UI for the fetch button
// after clicking fetch, hide the UI
document.getElementById('snippet-btn').addEventListener('click', function () {
    document.getElementById('url-container').classList.remove('d-none');
});
