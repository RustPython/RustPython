import * as rp from 'rustpython_wasm';
import pyCode from 'raw-loader!./main.py';

const vm = rp.vmStore.get('main');

vm.exec(pyCode);
