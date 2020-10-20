// MarkedJs: renders Markdown
// https://github.com/markedjs/marked
import marked from 'marked';

// KaTex: renders Math
// https://github.com/KaTeX/KaTeX
import katex from 'katex';
import 'katex/dist/katex.min.css';

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

function runPython(code, target, error) {
    try {
        rp.pyExec(code, {
            stdout: (output) => {
                target.innerHTML += output;
            }
        });
    } catch (err) {
        if (err instanceof WebAssembly.RuntimeError) {
            err = window.__RUSTPYTHON_ERROR || err;
        }
        error.textContent = err;
    }
}

// Evaluate javascript
function runJS(code) {
    return eval(code);
}



export { runPython, renderMarkdown, renderMath, runJS } 