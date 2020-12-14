import './style.css';
// Code Mirror
// https://github.com/codemirror/codemirror
import CodeMirror from 'codemirror';
import 'codemirror/mode/python/python';
import 'codemirror/mode/javascript/javascript';
import 'codemirror/mode/css/css';
import 'codemirror/mode/markdown/markdown';
import 'codemirror/mode/stex/stex';
import 'codemirror/addon/comment/comment';
import 'codemirror/lib/codemirror.css';
import 'codemirror/theme/ayu-mirage.css';

import { selectBuffer, openBuffer, newBuf } from './editor';

import { genericFetch } from './tools';

// parsing: copied from the iodide project
// https://github.com/iodide-project/iodide/blob/master/src/editor/iomd-tools/iomd-parser.js
import { iomdParser } from './parse';

// processing: execute/render editor's content
import {
    runPython,
    runJS,
    addCSS,
    checkCssStatus,
    renderMarkdown,
    renderMath,
    handlePythonError,
} from './process';

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

const error = document.getElementById('error');
const notebook = document.getElementById('rp-notebook');

// Code Editors
// There is a primary and secondary code editor
// By default only the primary is visible.
// On click of split view, secondary editor is visible
// Each editor can display multiple documents and doc types.
// the created ones are main/python/js/css
// user has the option to add their own documents.
// all new documents are python docs
// adapted/inspired from https://codemirror.net/demo/buffers.html
const primaryEditor = CodeMirror(document.getElementById('primary-editor'), {
    theme: 'ayu-mirage',
    lineNumbers: true,
    lineWrapping: true,
});

const secondaryEditor = CodeMirror(
    document.getElementById('secondary-editor'),
    {
        lineNumbers: true,
        lineWrapping: true,
    }
);

const buffers = {};

// list of buffers (displayed on UI as inline list item next to run)
const buffersList = document.getElementById('buffers-list');

// dropdown of buffers (visible on click of split view)
const buffersDropDown = document.getElementById('buffers-selection');

// By default open 3 buffers, main, tab1 and css
// TODO: add a JS option
// Params for OpenBuffer (buffers object, name of buffer to create, default content, type, link in UI 1, link in UI 2)
openBuffer(
    buffers,
    'main',
    '# python code or code blocks that start with %%py, %%md %%math.',
    'notebook',
    buffersDropDown,
    buffersList
);

openBuffer(
    buffers,
    'python',
    '# Python code',
    'python',
    buffersDropDown,
    buffersList
);

openBuffer(
    buffers,
    'js',
    '// Javascript code goes here',
    'javascript',
    buffersDropDown,
    buffersList
);

openBuffer(
    buffers,
    'css',
    '/* CSS goes here */',
    'css',
    buffersDropDown,
    buffersList
);

// select main buffer by default and set the main tab to active
selectBuffer(primaryEditor, buffers, 'main');
selectBuffer(secondaryEditor, buffers, 'main');
document
    .querySelector('ul#buffers-list li:first-child')
    .classList.add('active');

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

document.getElementById('run-btn').addEventListener('click', executeNotebook);

let pyvm = null;

// on click of run
// 1. add css stylesheet
// 2. get and run content of all tabs (including dynamically added ones)
// 3. run main tab.
async function executeNotebook() {
    // Clean the console and errors
    notebook.innerHTML = '';
    error.textContent = '';

    // get the content of the css editor
    // and add the css to the head
    // use dataset.status for a flag to know when to update
    let cssCode = buffers['css'].getValue();
    let cssStatus = checkCssStatus();
    switch (cssStatus) {
        case 'none':
            addCSS(cssCode);
            break;
        case 'modified':
            // remove the old style then add the new one
            document.getElementsByTagName('style')[0].remove();
            addCSS(cssCode);
            break;
        default:
        // do nothing
    }

    if (pyvm) {
        pyvm.destroy();
        pyvm = null;
    }
    pyvm = rp.vmStore.init('notebook_vm');

    // add some helpers for js/python code
    window.injectPython = (ns) => {
        for (const [k, v] of Object.entries(ns)) {
            pyvm.addToScope(k, v);
        }
    };
    window.pushNotebook = (elem) => {
        notebook.appendChild(elem);
    };
    window.handlePyError = (err) => {
        handlePythonError(error, err);
    };
    pyvm.setStdout((text) => {
        const para = document.createElement('p');
        para.appendChild(document.createTextNode(text));
        notebook.appendChild(para);
    });
    for (const el of ['h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p']) {
        pyvm.addToScope(el, (text) => {
            const elem = document.createElement(el);
            elem.appendChild(document.createTextNode(text));
            notebook.appendChild(elem);
        });
    }
    pyvm.addToScope('notebook_html', (html) => {
        notebook.innerHTML += html;
    });

    let jsCode = buffers['js'].getValue();
    await runJS(jsCode);

    // get all the buffers, except css, js and main
    // css is auto executed at the start
    // main is parsed then executed at the end
    // main can have md, math and python function calls
    let { css, main, js, ...pythonBuffers } = buffers;

    for (const [name] of Object.entries(pythonBuffers)) {
        let pythonCode = buffers[name].getValue();
        runPython(pyvm, pythonCode, error);
    }

    // now parse from the main editor

    // gets code from main editor
    let mainCode = buffers['main'].getValue();
    /* 
	Split code into chunks.
	Uses %%keyword or %% keyword as separator
	Returned object has: 
	    - chunkContent, chunkType, chunkId, 
	    - evalFlags, startLine, endLine 
	*/
    let parsedCode = iomdParser(mainCode);
    for (const chunk of parsedCode) {
        // For each type of chunk, do somthing
        // so far have py for python, md for markdown and math for math ;p
        let content = chunk.chunkContent;
        switch (chunk.chunkType) {
            // by default assume this is python code
            // so users don't have to type py manually
            case '':
            case 'py':
                runPython(pyvm, content, error);
                break;
            // TODO: fix how js is injected and ran
            case 'js':
                await runJS(content);
                break;
            case 'md':
                notebook.innerHTML += renderMarkdown(content);
                break;
            case 'math':
                notebook.innerHTML += renderMath(content);
                break;
            default:
            // do nothing when we see an unknown chunk for now
        }
    }
}

function updatePopup(type, message) {
    document.getElementById('popup').dataset.type = type;
    document.getElementById('popup-header').textContent = message;
}

// import button
// show a url input + fetch button
// takes a url where there is raw code
document
    .getElementById('popup-import')
    .addEventListener('click', async function () {
        let url = document.getElementById('popup-url').value;
        let type = document.getElementById('popup').dataset.type;
        let code = await genericFetch(url, type);
        primaryEditor.setValue(code);
    });

document.getElementById('import-code').addEventListener('click', function () {
    updatePopup('python', 'URL (raw text format)');
});

// click on an item in the list
CodeMirror.on(buffersList, 'click', function (e) {
    selectBuffer(primaryEditor, buffers, e.target.dataset.language);
});

// select an item in the dropdown
CodeMirror.on(buffersDropDown, 'change', function () {
    selectBuffer(
        secondaryEditor,
        buffers,
        buffersDropDown.options[buffersDropDown.selectedIndex].value
    );
});

// when css code editor changes
// update data attribute flag to modified
CodeMirror.on(buffers['css'], 'change', function () {
    let style = document.getElementsByTagName('style')[0];
    if (style) {
        style.dataset.status = 'modified';
    }
});

document
    .getElementById('buffers-list')
    .addEventListener('click', function (event) {
        let elem = document.querySelector('.active');
        if (elem) {
            elem.classList.remove('active');
        }
        event.target.classList.add('active');
    });

// new tab, new buffer
document.getElementById('new-tab').addEventListener('click', function () {
    newBuf(buffers, buffersDropDown, buffersList, primaryEditor);
});

// TODO: those three addEventListener can be re-written into one thing probably
document.getElementById('split-view').addEventListener('click', function () {
    document.getElementById('primary-editor').classList.remove('d-none');
    document.getElementById('secondary-editor').classList.remove('d-none');
});
document.getElementById('reader-view').addEventListener('click', function () {
    document.getElementById('primary-editor').classList.add('d-none');
    document.getElementById('secondary-editor').classList.add('d-none');
});
document.getElementById('default-view').addEventListener('click', function () {
    document.getElementById('primary-editor').classList.remove('d-none');
    document.getElementById('secondary-editor').classList.add('d-none');
});
