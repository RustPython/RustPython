use cpython::Python;
use cpython::ObjectProtocol; //for call method
use cpython::PyObject;
use cpython::PyDict;
use python27_sys::PyCodeObject;


//pub fn compile() -> PyObject {
pub fn compile(){
    let gil = Python::acquire_gil();
    let py = gil.python();

    let locals = PyDict::new(py);
    // TODO: read the filename from commandline
    //locals.set_item(py, "filename", "../tests/function.py").unwrap();

    let load_file = "\
import os
print(os.getcwd())
filename = '../tests/function.py'
with open(filename, 'rU') as f:\
    code = f.read()\
";
    py.run(load_file, None, Some(&locals)).unwrap();
    let code = py.eval("compile(code, \"foo\", \"exec\")", None, Some(&locals)).unwrap();
    //println!("{:?}", code.getattr(py, "co_name").unwrap());
    //println!("{:?}", code.getattr(py, "co_filename").unwrap());
    //println!("{:?}", code.getattr(py, "co_code").unwrap());
    //println!("{:?}", code.getattr(py, "co_freevars").unwrap());
    //println!("{:?}", code.getattr(py, "co_cellvars").unwrap());
    println!("{:?}", code.getattr(py, "co_consts").unwrap());
    //let consts =  code.getattr(py, "co_consts").unwrap();
    //println!("{:?}", consts.get_item(py, 0).unwrap().getattr(py, "co_code"));

}
