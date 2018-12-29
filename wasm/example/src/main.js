import * as py from 'rustpython_wasm';
import pyCode from 'raw-loader!./main.py';

fetch('https://github-trending-api.now.sh/repositories')
    .then(r => r.json())
    .then(repos => {
        const result = py.pyEval(pyCode, {
            vars: { repos }
        });
        alert(result);
    });
