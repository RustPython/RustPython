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

// copied from the iodide project
// https://github.com/iodide-project/iodide/blob/master/src/editor/iomd-tools/iomd-parser.js
import { iomdParser } from './parse';
import { runPython, runJS, addCSS, renderMarkdown, renderMath } from './process';
import { injectJS } from './utils';
import { selectBuffer, openBuffer , newBuf } from './editor';

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
// When clicking coding mode, secondary editor is visible
// Each editor can display multiple documents and doc types.
// the default created ones are main /python/js/css
// user has the option to add their own document. By default it is python
// adapted/inspired from https://codemirror.net/demo/buffers.html

const primaryEditor = CodeMirror(document.getElementById("primary-editor"), { theme: "ayu-mirage", lineNumbers: true ,  lineWrapping: true , indentUnit: 4  });
const secondaryEditor = CodeMirror(document.getElementById("secondary-editor"), { lineNumbers: true , lineWrapping: true , indentUnit: 4 });

const buffers = {};
const buffersList = document.getElementById("buffers-list");
const buffersDropDown = document.getElementById("buffers-selection");

openBuffer(buffers, "main", "# Write python code or use code blocks that start with %%py, %%js, %%md %%math.", "notebook", buffersDropDown, buffersList);
openBuffer(buffers, "python", "# Python code goes here", "python", buffersDropDown, buffersList);
openBuffer(buffers, "js", "// Javascript code go here", "javascript", buffersDropDown, buffersList);
openBuffer(buffers, "css", "/* CSS code goes here. */", "css", buffersDropDown, buffersList);

selectBuffer(primaryEditor, buffers, "main");
selectBuffer(secondaryEditor, buffers, "main");

CodeMirror.on(buffersList, "click", function (e) {
    selectBuffer(primaryEditor, buffers, e.target.dataset.language);
});

CodeMirror.on(buffersDropDown, "change", function () {
    selectBuffer(secondaryEditor, buffers, buffersDropDown.options[buffersDropDown.selectedIndex].value);
});

document.getElementById('new-tab').addEventListener('click', function () {
    newBuf(buffers, buffersDropDown, buffersList, primaryEditor);
});



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

// Parses what is the code editor
// either runs python or renders math or markdown
function parseCodeFromEditor() {

     // Clean the console and errors
    notebook.innerHTML = '';
    error.textContent = '';

    let css_code = buffers["css"].getValue();
    addCSS(css_code);
    console.log(css_code);

    // Read javascript code from the jsEditor
    // Inject JS into DOM, so that functions can be called from python
    // if there is an edit
    // detect and inject js code
    let js_code = buffers["js"].getValue();
    injectJS(js_code);
    console.log(js_code);

    // add loop and if conditions
    let python_code = buffers["python"].getValue();
    runPython(python_code, notebook, error);

    // gets code from main editor
    let main_code = buffers["main"].getValue();
    /* 
    Split code into chunks.
    Uses %%keyword or %% keyword as separator
    Returned object has: 
        - chunkContent, chunkType, chunkId, 
        - evalFlags, startLine, endLine 
    */
    let parsed_code = iomdParser(main_code);
    parsed_code.forEach(async (chunk) => {
        // For each type of chunk, do somthing
        // so far have py for python, md for markdown and math for math ;p
        let content = chunk.chunkContent;

        switch (chunk.chunkType) {
            // by default assume this is python code
            // so users don't have to type py manually
            case '':
            case 'py':
                runPython(content, notebook, error);
                break;
            case 'js':
                runJS(content);
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
    });
}



// import button
// show a url input + fetch button
// takes a url where there is raw code
// document.getElementById('popup-import').addEventListener('click', async function () {
//     const url = document.getElementById('popup-url').value;
//     const type = document.getElementById('popup').dataset.type;
//     const code = await genericFetch(url, type);
//     switch (type) {
//         case '':
//         case 'py':
//             pyEditor.setValue(code);
//             break;
//         case 'js':
//             jsEditor.setValue(code);
//             break;
//         default:
//         //do nothing
//     }

// });

// document.getElementById('import-code').addEventListener('click' , function() {
//     updatePopup('python', 'URL (raw text format)');
// });



document.getElementById('split-view').addEventListener('click', function() {
    document.getElementById('secondary-editor').classList.remove('d-none');
});