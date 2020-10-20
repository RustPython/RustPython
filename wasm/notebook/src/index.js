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
import 'codemirror/theme/base16-dark.css';

// copied from the iodide project
// https://github.com/iodide-project/iodide/blob/master/src/editor/iomd-tools/iomd-parser.js
import { iomdParser } from './parse';
import { runPython , runJS, renderMarkdown , renderMath } from './process';
import { genericFetch , injectJS } from './utils';
import { selectBuffer , openBuffer } from './editor'
let rp;

const error = document.getElementById('error');
const notebook = document.getElementById('rp-notebook');
let buffers = {};
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

// Parses what is the code editor
// either runs python or renders math or markdown
function parseCodeFromEditor() {
    
    let test = primaryEditor.getValue();
    // Clean the console and errors
    notebook.innerHTML = '';
    error.textContent = '';

    // Read javascript code from the jsEditor
    // Injsect JS into DOM, so that functions can be called from python
    // let js_code = jsEditor.getValue();
    // injectJS(js_code);

    // gets the code from code editor
    // let python_code = pyEditor.getValue();
    /* 
    Split code into chunks.
    Uses %%keyword or %% keyword as separator
    Returned object has: 
        - chunkContent, chunkType, chunkId, 
        - evalFlags, startLine, endLine 
    */
    let python_code = "print('hello world')";
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
                runPython(content, notebook, error);
                break;
            case 'js':
                runJS(content);
                break;
            case 'md':
                notebook.innerHTML += renderMarkdown(content);
                break;
            case 'math':
                notebook.innerHTML += renderMath(content, true);
                break;
            default:
            // do nothing when we see an unknown chunk for now
        }
    });
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
        case 'py':
            pyEditor.setValue(code);
            break;
        case 'js':
            jsEditor.setValue(code);
            break;
        default:
            //do nothing
    }
       
});

// document.getElementById('import-js-library').addEventListener('click' , function() {
//     updatePopup('javascript', 'URL/CDN of the Javascript library');
// }); 

// document.getElementById('import-code').addEventListener('click' , function() {
//     updatePopup('python', 'URL (raw text format)');
// });

document.getElementById('new-tab').addEventListener('click' , function() {
  newBuf();
});

// Tabbed Navigation
document.addEventListener('click', ({ target: { dataset: { id = '' } } }) => {
    if (id.length > 0) {
        document.querySelectorAll('.tab').forEach(t =>  t.classList.add('d-none'));
        document.querySelector(`#${id}`).classList.remove('d-none');
    }
});

// function updatePopup(type, message) {
//     document.getElementById('popup').dataset.type =  type ;
//     document.getElementById('popup-header').textContent = message;
// }
 
let buffersList = document.getElementById("buffers-list");

CodeMirror.on(buffersList, "click", function(e) {
    selectBuffer(primaryEditor, buffers, e.target.dataset.language);
});

let buffersDropDown = document.getElementById("buffers-selection");
    CodeMirror.on(buffersDropDown, "change", function() {
    selectBuffer(secondaryEditor, buffers, buffersDropDown.options[buffersDropDown.selectedIndex].value);
});

  
  function newBuf() {
    let name = prompt("Name for the buffer", "*scratch*");
    if (name == null) return;
    if (buffers.hasOwnProperty(name)) {
      alert("There's already a buffer by that name.");
      return;
    }
    openBuffer(buffers, name, "", "javascript" , buffersDropDown , buffersList);
    selectBuffer( primaryEditor , buffers, name);
    let sel = buffersDropDown;
    sel.value = name;
  }


openBuffer(buffers, "main",  "", "notebook" ,  buffersDropDown , buffersList);
openBuffer(buffers, "python", "# Python code goes here", "python" ,  buffersDropDown , buffersList);
openBuffer(buffers, "js", "// Javascript goes here", "javascript" ,  buffersDropDown , buffersList);
openBuffer(buffers, "css", "/* CSS goes here */", "css" ,  buffersDropDown , buffersList);


var primaryEditor = CodeMirror(document.getElementById("primary-editor"), {lineNumbers: true});
selectBuffer(primaryEditor, buffers,  "main");
var secondaryEditor = CodeMirror(document.getElementById("secondary-editor"), {lineNumbers: true});
selectBuffer(secondaryEditor, buffers,  "main");
