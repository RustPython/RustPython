import * as rp from 'rustpython_wasm';

// so people can play around with it
window.rp = rp;

function runCodeFromTextarea(_) {
    const consoleElement = document.getElementById('console');
    const errorElement = document.getElementById('error');

    // Clean the console and errors
    consoleElement.value = '';
    errorElement.textContent = '';

    const code = document.getElementById('code').value;
    try {
        rp.run_from_textbox(code);
    } catch (e) {
        errorElement.textContent = e;
        console.error(e);
    }
}

document
    .getElementById('run-btn')
    .addEventListener('click', runCodeFromTextarea);

runCodeFromTextarea(); // Run once for demo
