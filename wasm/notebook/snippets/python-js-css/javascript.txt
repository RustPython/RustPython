// Javascript code goes here

injectPython({
    // injectPython functions take the positional arguments as
    // normal function args, and kwargs as the `this` variable
    add_text_input() {
        const input = document.createElement('input');
        pushNotebook(input);
        return () => input.value;
    },
    add_button(buttonText, cb) {
        const do_button = (callback) => {
            const btn = document.createElement('button');
            btn.innerHTML = buttonText;
            btn.addEventListener('click', () => {
                try {
                    // python functions passed to js have a signature
                    // of ([args...], {kwargs...}) => any
                    callback([], {});
                } catch (err) {
                    // puts the traceback in the error box
                    handlePyError(err);
                }
            });
            pushNotebook(btn);
        };

        if (cb == null) {
            // to allow using as a decorator
            return do_button;
        } else {
            do_button(cb);
        }
    },
    add_output() {
        const resultDiv = document.createElement('div');
        resultDiv.classList.add('result');
        pushNotebook(resultDiv);
        return (value) => {
            resultDiv.innerHTML = value;
        };
    },
});
