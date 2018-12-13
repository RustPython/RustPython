import * as rp from "rustpython_wasm";

function runCodeFromTextarea(_) {
  const consoleElement = document.getElementById('console');
  // Clean the console
  consoleElement.value = '';

  const code = document.getElementById('code').value;
  try {
    if (!code.endsWith('\n')) { // HACK: if the code doesn't end with newline it crashes.
      rp.run_code(code + '\n');
      return;
    }

    rp.run_code(code);

  } catch(e) {
    consoleElement.value = 'Execution failed. Please check if your Python code has any syntax error.';
    console.error(e);
  }

}

document.getElementById('run-btn').addEventListener('click', runCodeFromTextarea);

runCodeFromTextarea(); // Run once for demo
