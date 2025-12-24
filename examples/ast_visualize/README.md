# AST Visualize Example

This example shows how to render a Python AST as a structured tree or a
Graphviz DOT file using RustPython's `ast` module.

## Run

Tree view (default):

```
./target/release/rustpython examples/ast_visualize/ast_view.py --file examples/ast_visualize/sample.py
```

Dump view (ast.dump):

```
./target/release/rustpython examples/ast_visualize/ast_view.py --file examples/ast_visualize/sample.py --format dump
```

Graphviz DOT output:

```
./target/release/rustpython examples/ast_visualize/ast_view.py --file examples/ast_visualize/sample.py --format dot --output ast.dot
```

## Graphviz 安装与渲染

macOS (Homebrew):

```
brew install graphviz
```

macOS (Conda):

```
conda install -c conda-forge graphviz
```

Ubuntu/Debian:

```
sudo apt-get update
sudo apt-get install graphviz
```

Fedora:

```
sudo dnf install graphviz
```

Arch:

```
sudo pacman -S graphviz
```

Windows (Chocolatey):

```
choco install graphviz
```

Windows (Scoop):

```
scoop install graphviz
```

安装完成后将 DOT 渲染为图片：

```
dot -Tpng ast.dot -o ast.png
```

打开图片：

macOS:

```
open ast.png
```

Linux:

```
xdg-open ast.png
```

Windows:

```
start ast.png
```

## Example Output

Tree view:

```
`-- Module
    |-- FunctionDef name=add
    |   |-- arguments
    |   |   |-- arg arg=a
    |   |   `-- arg arg=b
    |   `-- Return
    |       `-- BinOp
    |           |-- Name id=a ctx=Load
    |           |-- Add
    |           `-- Name id=b ctx=Load
    |-- Assign targets=list[1]
    |   |-- Name id=result ctx=Store
    |   `-- Call func=Name
    |       |-- Name id=add ctx=Load
    |       |-- Constant value=1
    |       `-- Constant value=2
    `-- If
        |-- Compare
        |   |-- Name id=result ctx=Load
        |   |-- Gt
        |   `-- Constant value=2
        `-- Expr
            `-- Call func=Name
                |-- Name id=print ctx=Load
                `-- Constant value='ok'
```

Dump view (excerpt):

```
Module(
  body=[
    FunctionDef(
      name='add',
      args=arguments(
        posonlyargs=[],
        args=[
          arg(arg='a'),
          arg(arg='b')],
        kwonlyargs=[],
        kw_defaults=[],
        defaults=[]),
      body=[
        Return(
          value=BinOp(
            left=Name(id='a', ctx=Load()),
            op=Add(),
            right=Name(id='b', ctx=Load())))],
      decorator_list=[]),
    Assign(
      targets=[
        Name(id='result', ctx=Store())],
      value=Call(
        func=Name(id='add', ctx=Load()),
        args=[
          Constant(value=1),
          Constant(value=2)],
        keywords=[])),
    If(
      test=Compare(
        left=Name(id='result', ctx=Load()),
        ops=[
          Gt()],
        comparators=[
          Constant(value=2)]),
      body=[
        Expr(
          value=Call(
            func=Name(id='print', ctx=Load()),
            args=[
              Constant(value='ok')],
            keywords=[]))],
      orelse=[])],
  type_ignores=[])
```

## Notes

- Use `--code` to pass inline code.
- Use `--attrs` to include line/column info.
- If you render DOT, use Graphviz (dot) to convert it to an image.
