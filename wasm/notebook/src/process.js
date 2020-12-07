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
function renderMath(math) {
    // TODO: definetly add error handling.
    return katex.renderToString(math, {
        macros: { '\\f': '#1f(#2)' },
    });
}

function runPython(pyvm, code, error) {
    try {
        pyvm.exec(code);
    } catch (err) {
        handlePythonError(error, err);
    }
}

function handlePythonError(errorElem, err) {
    if (err instanceof WebAssembly.RuntimeError) {
        err = window.__RUSTPYTHON_ERROR || err;
    }
    errorElem.textContent = err;
}

function addCSS(code) {
    let style = document.createElement('style');
    style.type = 'text/css';
    style.innerHTML = code;
    // add a data attribute to check if css already loaded
    style.dataset.status = 'loaded';
    document.getElementsByTagName('head')[0].appendChild(style);
}

function checkCssStatus() {
    let style = document.getElementsByTagName('style')[0];
    if (!style) {
        return 'none';
    } else {
        return style.dataset.status;
    }
}

async function runJS(code) {
    const script = document.createElement('script');
    const doc = document.body || document.documentElement;
    const blob = new Blob([code], { type: 'text/javascript' });
    const url = URL.createObjectURL(blob);
    script.src = url;
    const scriptLoaded = new Promise((resolve) => {
        script.addEventListener('load', resolve);
    });
    doc.appendChild(script);
    try {
        URL.revokeObjectURL(url);
        doc.removeChild(script);
        await scriptLoaded;
    } catch (e) {
        // ignore if body is changed and script is detached
        console.log(e);
    }
}

export {
    runPython,
    handlePythonError,
    runJS,
    renderMarkdown,
    renderMath,
    addCSS,
    checkCssStatus,
};
