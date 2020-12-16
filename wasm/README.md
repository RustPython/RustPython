# Compiling to webassembly

At this stage RustPython only has preliminary support for web assembly. The
instructions here are intended for developers or those wishing to run a toy
example.

## Setup

To get started, install
[wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) and `npm`.
([wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/whirlwind-tour/basic-usage.html)
should be installed by `wasm-pack`. if not, install it yourself)

## Build

Move into the `wasm` directory. This directory contains a library crate for
interop with python to rust to js and back in `wasm/lib`, the demo website found
at https://rustpython.github.io/demo in `wasm/demo`, and an example of how to
use the crate as a library in one's own JS app in `wasm/example`.

```sh
cd wasm
```

Go to the demo directory. This is the best way of seeing the changes made to
either the library or the JS demo, as the `rustpython_wasm` module is set to the
global JS variable `rp` on the website.

```sh
cd demo
```

Now, start the webpack development server. It'll compile the crate and then the
demo app. This will likely take a long time, both the wasm-pack portion and the
webpack portion (from after it says "Your crate has been correctly compiled"),
so be patient.

```sh
npm run dev
```

You can now open the webpage on https://localhost:8080 and Python code in either
the text box or browser devtools with:

```js
rp.pyEval(
    `
print(js_vars['a'] * 9)
`,
    {
        vars: {
            a: 9,
        },
    }
);
```

Alternatively, you can run `npm run build` to build the app once, without
watching for changes, or `npm run dist` to build the app in release mode, both
for the crate and webpack.

## Updating the demo

If you wish to update the WebAssembly demo,
[open a pull request](https://github.com/RustPython/RustPython/compare/release...master)
to merge `master` into the `release` branch. This will trigger a Travis build
that updates the demo page.
