import CodeMirror from 'codemirror';

CodeMirror.defineMode('notebook', function (config, _parserConfig) {
    const nullMode = CodeMirror.getMode(config, 'text/plain');
    const python = CodeMirror.getMode(config, 'python');
    const markdown = CodeMirror.getMode(config, 'markdown');
    const latex = CodeMirror.getMode(config, 'text/x-latex');
    const javascript = CodeMirror.getMode(config, 'javascript');
    const modeMap = {
        py: python,
        md: markdown,
        math: latex,
        js: javascript,
    };
    return {
        startState() {
            return {
                mode: python,
                modeState: python.startState(),
                chunkStart: false,
            };
        },
        token(stream, state) {
            if (stream.sol() && stream.match('%%')) {
                stream.eatSpace();
                state.chunkStart = true;
                return 'keyword';
            }
            if (state.chunkStart) {
                const m = stream.match(/[\w\-]+/);
                const name = m && m[0];
                const mode = (state.mode = modeMap[name] || nullMode);
                state.modeState = mode.startState ? mode.startState() : null;
                state.chunkStart = false;
                return 'keyword';
            }
            const { mode, modeState } = state;
            return mode.token(stream, modeState);
        },
        indent(state, textAfter, line) {
            const { mode, modeState } = state;
            if (mode.indent) return mode.indent(modeState, textAfter, line);
        },
        innerMode(state) {
            const { mode, modeState } = state;
            return { mode, state: modeState };
        },
    };
});

CodeMirror.defineMIME('text/x-notebook', 'notebook');

function selectBuffer(editor, buffers, name) {
    var buf = buffers[name];
    if (buf.getEditor()) buf = buf.linkedDoc({ sharedHist: true });
    var old = editor.swapDoc(buf);
    var linked = old.iterLinkedDocs(function (doc) {
        linked = doc;
    });
    if (linked) {
        // Make sure the document in buffers is the one the other view is looking at
        for (var name in buffers)
            if (buffers[name] == old) buffers[name] = linked;
        old.unlinkDoc(linked);
    }
    editor.focus();
    // console.log(editor.getValue());
}

function openBuffer(buffers, name, text, mode, buffersDropDown, buffersList) {
    buffers[name] = CodeMirror.Doc(text, mode);
    let opt = document.createElement('option');
    opt.appendChild(document.createTextNode(name));
    buffersDropDown.appendChild(opt);

    let li = document.createElement('li');
    li.appendChild(document.createTextNode(name));
    li.dataset.language = name;
    buffersList.appendChild(li);
}

function newBuf(buffers, buffersDropDown, buffersList, primaryEditor) {
    let name = prompt('Name your tab', '*scratch*');
    if (name == null) return;
    if (buffers.hasOwnProperty(name)) {
        alert("There's already a buffer by that name.");
        return;
    }
    openBuffer(buffers, name, '', 'python', buffersDropDown, buffersList);
    selectBuffer(primaryEditor, buffers, name);
    let sel = buffersDropDown;
    sel.value = name;
}

export { selectBuffer, openBuffer, newBuf };
