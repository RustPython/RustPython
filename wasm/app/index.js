import * as rp from "rustpython_wasm";

function runCodeFromTextarea(_) {
  // Clean the console
  document.getElementById('console').value = '';

  const code = document.getElementById('code').value;
  if (!code.endsWith('\n')) { // HACK: if the code doesn't end with newline it crashes.
    rp.run_code(code + '\n');
    return;
  }
  rp.run_code(code);
}
document.getElementById('run-btn').addEventListener('click', runCodeFromTextarea);

runCodeFromTextarea(); // Run once for demo
