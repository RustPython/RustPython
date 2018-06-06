class: center, middle
##Python Interpreter in Rust
###Introduction
#### 2017/3/28
#### Shing Lyu


???
top, middle, bottom
left, center, right

---
name: toc
###Agenda
1. Category 
1. Category 
1. Category 
1. Category 
1. Category 
1. Category 
1. Category 

???
This is a template
---

### Python's architecture
* Interpreted
* Garbage Collected
* Compiler => bytecode => VM

---
background-image: url('pic/ice-cream.jpg')
class: bleed
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
### Why rewriting Python in Rust?
* Memory safety

* Learn about Python internal
* Learn real world Rust

---
### Implementation strategy
* Focus on the VM first, then the compiler
  * Reuse the Python built-in compiler to generate bytecode
* Basic arithmetics
* Control flows (require JUMP)
* Function call (require call stack)
* Built-in functions (require native code)
* Run popular libraries


---
### References
* [`dis` documentation](https://docs.python.org/3.4/library/dis.html)
* [byterun](http://www.aosabook.org/en/500L/a-python-interpreter-written-in-python.html)
* [byterun (GitHub)](https://github.com/nedbat/byterun/)
* [cpython source code](https://github.com/python/cpython)

---
### How Python VM works
* Stack machine
* Accepts Python bytecode
* `python -m dis source.py`

---

### A simple Python code

```
#!/usr/bin/env python3
print(1+1)
```

We run `python3 -m dis source.py`

---

### The bytecode

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
1. argument = stack.pop() (argument == 2)
2. function = stack.top() (function == print)
3. call print(2)

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
### LOAD_CONST 1
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

### Next step
* Make it run a small but popular tool/library
* Implement the parser
* Performance benchmarking
