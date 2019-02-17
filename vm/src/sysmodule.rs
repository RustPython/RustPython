use obj::objtype;
use pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use std::rc::Rc;
use std::{env, mem};
use vm::VirtualMachine;

/*
 * The magic sys module.
 */

fn argv(ctx: &PyContext) -> PyObjectRef {
    let mut argv: Vec<PyObjectRef> = env::args().map(|x| ctx.new_str(x)).collect();
    argv.remove(0);
    ctx.new_list(argv)
}

fn getframe(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    if let Some(frame) = &vm.current_frame {
        Ok(frame.clone())
    } else {
        panic!("Current frame is undefined!")
    }
}

fn sys_getrefcount(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(object, None)]);
    let size = Rc::strong_count(&object);
    Ok(vm.ctx.new_int(size))
}

fn sys_getsizeof(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(object, None)]);
    // TODO: implement default optional argument.
    let size = mem::size_of_val(&object.borrow());
    Ok(vm.ctx.new_int(size))
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let path_list = match env::var_os("PYTHONPATH") {
        Some(paths) => env::split_paths(&paths)
            .map(|path| {
                ctx.new_str(
                    path.to_str()
                        .expect("PYTHONPATH isn't valid unicode")
                        .to_string(),
                )
            })
            .collect(),
        None => vec![],
    };
    let path = ctx.new_list(path_list);

    let modules = ctx.new_dict();

    let sys_name = "sys";
    let sys_doc = "This module provides access to some objects used or maintained by the
interpreter and to functions that interact strongly with the interpreter.

Dynamic objects:

argv -- command line arguments; argv[0] is the script pathname if known
path -- module search path; path[0] is the script directory, else ''
modules -- dictionary of loaded modules

displayhook -- called to show results in an interactive session
excepthook -- called to handle any uncaught exception other than SystemExit
  To customize printing in an interactive session or to install a custom
  top-level exception handler, assign other functions to replace these.

stdin -- standard input file object; used by input()
stdout -- standard output file object; used by print()
stderr -- standard error object; used for error messages
  By assigning other file objects (or objects that behave like files)
  to these, it is possible to redirect all of the interpreter's I/O.

last_type -- type of last uncaught exception
last_value -- value of last uncaught exception
last_traceback -- traceback of last uncaught exception
  These three are only available in an interactive session after a
  traceback has been printed.

Static objects:

builtin_module_names -- tuple of module names built into this interpreter
copyright -- copyright notice pertaining to this interpreter
exec_prefix -- prefix used to find the machine-specific Python library
executable -- absolute path of the executable binary of the Python interpreter
float_info -- a struct sequence with information about the float implementation.
float_repr_style -- string indicating the style of repr() output for floats
hash_info -- a struct sequence with information about the hash algorithm.
hexversion -- version information encoded as a single integer
implementation -- Python implementation information.
int_info -- a struct sequence with information about the int implementation.
maxsize -- the largest supported length of containers.
maxunicode -- the value of the largest Unicode code point
platform -- platform identifier
prefix -- prefix used to find the Python library
thread_info -- a struct sequence with information about the thread implementation.
version -- the version of this interpreter as a string
version_info -- version information as a named tuple
__stdin__ -- the original stdin; don't touch!
__stdout__ -- the original stdout; don't touch!
__stderr__ -- the original stderr; don't touch!
__displayhook__ -- the original displayhook; don't touch!
__excepthook__ -- the original excepthook; don't touch!

Functions:

displayhook() -- print an object to the screen, and save it in builtins._
excepthook() -- print an exception and its traceback to sys.stderr
exc_info() -- return thread-safe information about the current exception
exit() -- exit the interpreter by raising SystemExit
getdlopenflags() -- returns flags to be used for dlopen() calls
getprofile() -- get the global profiling function
getrefcount() -- return the reference count for an object (plus one :-)
getrecursionlimit() -- return the max recursion depth for the interpreter
getsizeof() -- return the size of an object in bytes
gettrace() -- get the global debug tracing function
setcheckinterval() -- control how often the interpreter checks for events
setdlopenflags() -- set the flags to be used for dlopen() calls
setprofile() -- set the global profiling function
setrecursionlimit() -- set the max recursion depth for the interpreter
settrace() -- set the global debug tracing function
";
    let sys_mod = ctx.new_module(&sys_name, ctx.new_scope(None));

    ctx.set_item(&modules, sys_name, sys_mod.clone());

    ctx.set_attr(&sys_mod, "modules", modules);
    ctx.set_attr(&sys_mod, "argv", argv(ctx));
    ctx.set_attr(&sys_mod, "getrefcount", ctx.new_rustfunc(sys_getrefcount));
    ctx.set_attr(&sys_mod, "getsizeof", ctx.new_rustfunc(sys_getsizeof));
    ctx.set_attr(&sys_mod, "maxsize", ctx.new_int(std::usize::MAX));
    ctx.set_attr(&sys_mod, "path", path);
    ctx.set_attr(&sys_mod, "ps1", ctx.new_str(">>>>> ".to_string()));
    ctx.set_attr(&sys_mod, "ps2", ctx.new_str("..... ".to_string()));
    ctx.set_attr(&sys_mod, "__doc__", ctx.new_str(sys_doc.to_string()));
    ctx.set_attr(&sys_mod, "_getframe", ctx.new_rustfunc(getframe));

    sys_mod
}
