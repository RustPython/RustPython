# RustPython Notebook

The RustPython Notebook is a **toy** notebook inspired by the now inactive Iodide project ([https://alpha.iodide.io/](https://alpha.iodide.io/)).

Here is how it looks like:  

![notebook](./screenshot.png)

You can use the notebook to experiment with using Python and Javascript in the browser together.   

The main use case is for scientific communication where you can have:
- text or thesis in markdown,
- math with Tex, 
- a model or analysis written in python, 
- a user interface and interactive visualization with JS.

The Notebook loads python in your browser (so you don't have to install it) then let yous play with those languages.

Using Javascript in the browser can play to JS strength but it is also a workaround since RustPython doesn't fully implement DOM/WebAPI functionality.

To read more about the reasoning behind certain features, check the blog on [https://rustpython.github.io/blog](https://rustpython.github.io/blog)

## Sample notebooks

Sample notebooks are under `snippets`

-  `snippets/python-markdown-math.txt`: python, markdown and math
-  `snippets/python-js.txt`, adds javascript
-  `snippets/python-js-css-md/` adds styling with css in separate, more organized files.

## How to use

- Run locally with `npm run dev`
- Build with `npm run dist`

## Wish list / TO DO

- Better javascript support
- Collaborative peer-to-peer editing with WebRTC. Think Google Doc or Etherpad editing but for code in the browser
- `%%load` command for dynamically adding javascript libraries or css framework
- Clean up and organize the code. Seriously rethink if we want to make it more than a toy.