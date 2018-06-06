class: center, middle
##Python Interpreter in Rust
#### 2017/8/6, COSCUP
#### Shing Lyu


???
top, middle, bottom
left, center, right

---
### About Me
* 呂行 Shing Lyu
* Mozilla engineer
* Servo team

![rust_and_servo](pic/rust-servo.png)
---
### Python's architecture
* Interpreted
* Garbage Collected
* Compiler => bytecode => VM

![python](pic/python-logo.png)

---
background-image: url('pic/ice-cream.jpg')
class: center, middle, bleed, text-bg
# Flavors


---
### Python Flavors
* CPython (THE python)
* Jython (JVM)
* IronPython (.NET)
* Pypy
* Educational
  * Byterun
  * Jsapy (JS)
  * Brython (Python in browser)

---
class: center, middle

# Why & How?
---
### Why rewriting Python in Rust?
* Memory safety (?)
* Learn about Python's internal
* Learn to write Rust from scratch
* FUN!

---
### Implementation strategy
* Mostly follow CPython 3.6
* Focus on the VM first, then the compiler
  * Use the Python built-in compiler to generate bytecode
* Focus on learning rather than usability

---
### Milestones
* Basic arithmetics
* Variables
* Control flows (require JUMP)
* Function call (require call stack)
* Built-in functions (require native code)
* Run Python tutorial example code <= We're here
* Exceptions
* GC
* Run popular libraries


---
class: center, middle, bleed, text-bg
background-image: url('pic/car_cutaway.jpg')
# Python Internals

---
### How Python VM works
* Stack machine
  * Call stack and frames
  * Has a NAMES list and CONSTS list
  * Has a STACK as workspace
* Accepts Python bytecode
* `python -m dis source.py`

---

### A simple Python code

```
#!/usr/bin/env python3
print(1+1)
```

Running `python3 -m dis source.py` gives us

```
  1    0 LOAD_NAME           0 (print)
       3 LOAD_CONST          2 (2)
       6 CALL_FUNCTION       1 (1 positional, 0 keyword pair)
       9 POP_TOP
      10 LOAD_CONST          1 (None)
      13 RETURN_VALUE

```

---

### LOAD_NAME "print"
* NAMES = ["print"]
* CONSTS = [None, 2]
* STACK:

```
 |                    |
 | print (native code)|
 +--------------------+
```
---
### LOAD_CONST 2
* NAMES = ["print"]
* CONSTS = [None, 2]
* STACK:

```
 |                    |
 |          2         |
 | print (native code)|
 +--------------------+
```

---

### CALL_FUNCTION 1
1. `argument = stack.pop()` (argument == 2)
2. `function = stack.top()` (function == print)
3. call `print(2)`

* NAMES = ["print"]
* CONSTS = [None, 2]
* STACK:

```
 |                    |
 | print (native code)|
 +--------------------+
```
---
### POP_TOP
* NAMES = ["print"]
* CONSTS = [None, 2]
* STACK:

```
 |                    |
 |     (empty)        |
 +--------------------+
```

---
### LOAD_CONST None
* NAMES = ["print"]
* CONSTS = [None, 2]
* STACK:

```
 |                    |
 |       None         |
 +--------------------+
```

---
### RETURN_VALUE

(returns top of stack == None)

---

class: center, middle, bleed, text-bg
background-image: url('pic/electronic_parts.jpg')
# Technical Detail

---

### Bytecode format
* `dis` output format is for human reader
* Implementing a `dis` format parser is a waste of time
* Emit JSON bytecode using the [bytecode](https://pypi.python.org/pypi/bytecode/0.5) module

```

code = compile(f,...)  # Python built-in, return a Code object

bytecode.Bytecode()
   .from_code(code)
   .to_concrete_bytecode()
```
* Load into Rust using `serde_json`
---

### Types
* Everything is a `PyObject` in CPython
* We'll need that class hierarchy eventually
* Use a Rust `enum` for now

```
pub enum NativeType{
    NoneType,
    Boolean(bool),
    Int(i32),
    Str(String),
    Tuple(Vec<NativeType>),
    ...
}
```

---

### Testing
* `assert` is essential to for unittests
* `assert` raises `AssertionError`
* Use `panic!()` before we implement exception

```
assert 1 == 1
```
```
  1           0 LOAD_CONST               0 (1)
              3 LOAD_CONST               0 (1)
              6 COMPARE_OP               2 (==)
              9 POP_JUMP_IF_TRUE        18
             12 LOAD_GLOBAL              0 (AssertionError)
             15 RAISE_VARARGS            1
        >>   18 LOAD_CONST               1 (None)
             21 RETURN_VALUE
```

---
### Native Function

* e.g. `print()`

```
pub enum NativeType {
    NativeFunction(fn(Vec<NativeType>) -> NativeType),
    ...
}

match stack.pop() {
    NativeFunction(func) => return_val = func(),
    _ => ...
}

```

---

### Next steps
* Exceptions
* Make it run a small but popular tool/library
* Implement the parser
* Figure out garbage collection
* Performance benchmarking

---
### Contribute

## https://github.com/shinglyu/RustPython

![qr_code](pic/repo_QR.png)

---
class: middle, center

# Thank you

---

### References
* [`dis` documentation](https://docs.python.org/3.4/library/dis.html)
* [byterun](http://www.aosabook.org/en/500L/a-python-interpreter-written-in-python.html)
* [byterun (GitHub)](https://github.com/nedbat/byterun/)
* [cpython source code](https://github.com/python/cpython)

