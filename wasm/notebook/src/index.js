import './style.css';

// Code Mirror 
// https://github.com/codemirror/codemirror
import CodeMirror from 'codemirror';
import 'codemirror/mode/python/python';
import 'codemirror/mode/javascript/javascript';
import 'codemirror/mode/markdown/markdown';
import 'codemirror/mode/stex/stex';
import 'codemirror/addon/comment/comment';
import 'codemirror/lib/codemirror.css';
import 'codemirror/theme/base16-dark.css';

// MarkedJs: renders Markdown
// https://github.com/markedjs/marked
import marked from 'marked';

// KaTex: renders Math
// https://github.com/KaTeX/KaTeX
import katex from 'katex';
import 'katex/dist/katex.min.css';

// copied from the iodide project
// https://github.com/iodide-project/iodide/blob/master/src/editor/iomd-tools/iomd-parser.js
import { iomdParser } from './parser';
import { genericFetch } from './utils';
import { inject } from './utils';

let rp;
let js_vars = {};

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
const pyEditor = CodeMirror(document.getElementById('python-code-editor'), {
    extraKeys: {
        'Ctrl-Enter': parseCodeFromEditor,
        'Cmd-Enter': parseCodeFromEditor,
        'Shift-Tab': 'indentLess',
        'Ctrl-/': 'toggleComment',
        'Cmd-/': 'toggleComment',
        Tab: (editor) => {
            var spaces = Array(editor.getOption('indentUnit') + 1).join(' ');
            pyEditor.replaceSelection(spaces);
        }
    },
    lineNumbers: true,
    mode: 'text/x-notebook',
    indentUnit: 4,
    autofocus: true,
    lineWrapping: true,
});

// JS Code Editor with dark theme
const jsEditor = CodeMirror(document.getElementById('javascript-code-editor'), {
    lineNumbers: false,
    indentUnit: 4,
    mode: 'text/javascript',
    theme: 'base16-dark',
    lineWrapping: true
});

// Parses what is the code editor
// either runs python or renders math or markdown
function parseCodeFromEditor() {
    // Clean the console and errors
    notebook.innerHTML = '';
    error.textContent = '';

    // Read javascript code from the jsEditor
    // Injsect JS into DOM, so that functions can be called from python
    let js_code = jsEditor.getValue();
    inject(js_code);

    // gets the code from code editor
    let python_code = pyEditor.getValue();
    /* 
    Split code into chunks.
    Uses %%keyword or %% keyword as separator
    Returned object has: 
        - chunkContent, chunkType, chunkId, 
        - evalFlags, startLine, endLine 
    */
    let parsed_code = iomdParser(python_code);

    parsed_code.forEach(async (chunk) => {
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
            case 'js':
                runJS(content);
                break;
            case 'math':
                notebook.innerHTML += renderMath(content, true);
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
            vars: js_vars
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

// Evaluate javascript
function runJS(content) {
    eval(content);
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
document.getElementById('popup-import').addEventListener('click', async function () {
    
    const url = document.getElementById('popup-url').value;
    const type = document.getElementById('popup').dataset.type;  
    const code = await genericFetch(url , type);
    switch (type) {
        case '':
        case 'python':
            pyEditor.setValue(code);
            break;
        case 'javascript':
            jsEditor.setValue(code);
            break;
        default:
            //do nothing
    }
       
});

document.getElementById('import-js-library').addEventListener('click' , function() {
    updatePopup('javascript', 'URL/CDN of the Javascript library');
}); 

document.getElementById('import-code').addEventListener('click' , function() {
    updatePopup('python', 'URL (raw text format)');
});

// Tabbed Navigation
document.addEventListener('click', ({ target: { dataset: { id = '' } } }) => {
    if (id.length > 0) {
        document.querySelectorAll('.tab').forEach(t =>  t.classList.add('d-none'));
        document.querySelector(`#${id}`).classList.remove('d-none');
    }
});

function updatePopup(type, message) {
    document.getElementById('popup').dataset.type =  type ;
    document.getElementById('popup-header').textContent = message;
}