use std::cell::{Cell, RefCell};

use super::objbool;
use super::objint;
use super::objsequence::{
    get_elements, get_elements_cell, get_item, seq_equal, seq_ge, seq_gt, seq_le, seq_lt, seq_mul,
    PySliceableSequence,
};
use super::objstr;
use super::objtype;
use crate::pyobject::{
    IdProtocol, OptionalArg, PyContext, PyFuncArgs, PyIteratorValue, PyObject, PyObjectRef, PyRef,
    PyResult, PyValue, TypeProtocol,
};
use crate::vm::{ReprGuard, VirtualMachine};
use num_traits::ToPrimitive;
use std::fmt;

#[derive(Default)]
pub struct PyList {
    // TODO: shouldn't be public
    pub elements: RefCell<Vec<PyObjectRef>>,
}

impl fmt::Debug for PyList {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("list")
    }
}

impl From<Vec<PyObjectRef>> for PyList {
    fn from(elements: Vec<PyObjectRef>) -> Self {
        PyList {
            elements: RefCell::new(elements),
        }
    }
}

impl PyValue for PyList {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.list_type()
    }
}

pub type PyListRef = PyRef<PyList>;

impl PyListRef {
    pub fn append(self, x: PyObjectRef, _vm: &mut VirtualMachine) {
        self.elements.borrow_mut().push(x);
    }

    fn extend(self, x: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<()> {
        let mut new_elements = vm.extract_elements(&x)?;
        self.elements.borrow_mut().append(&mut new_elements);
        Ok(())
    }

    fn insert(self, position: isize, element: PyObjectRef, _vm: &mut VirtualMachine) {
        let mut vec = self.elements.borrow_mut();
        let vec_len = vec.len().to_isize().unwrap();
        // This unbounded position can be < 0 or > vec.len()
        let unbounded_position = if position < 0 {
            vec_len + position
        } else {
            position
        };
        // Bound it by [0, vec.len()]
        let position = unbounded_position.max(0).min(vec_len).to_usize().unwrap();
        vec.insert(position, element.clone());
    }

    fn add(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let e1 = self.elements.borrow();
            let e2 = get_elements(&other);
            let elements = e1.iter().chain(e2.iter()).cloned().collect();
            Ok(vm.ctx.new_list(elements))
        } else {
            Err(vm.new_type_error(format!("Cannot add {} and {}", self.as_object(), other)))
        }
    }

    fn iadd(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            self.elements
                .borrow_mut()
                .extend_from_slice(&get_elements(&other));
            Ok(self.into_object())
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn clear(self, _vm: &mut VirtualMachine) {
        self.elements.borrow_mut().clear();
    }

    fn copy(self, vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(self.elements.borrow().clone())
    }

    fn len(self, _vm: &mut VirtualMachine) -> usize {
        self.elements.borrow().len()
    }

    fn reverse(self, _vm: &mut VirtualMachine) {
        self.elements.borrow_mut().reverse();
    }

    fn getitem(self, needle: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        get_item(
            vm,
            self.as_object(),
            &self.elements.borrow(),
            needle.clone(),
        )
    }

    fn iter(self, vm: &mut VirtualMachine) -> PyObjectRef {
        PyObject::new(
            PyIteratorValue {
                position: Cell::new(0),
                iterated_obj: self.into_object(),
            },
            vm.ctx.iter_type(),
        )
    }

    fn setitem(self, key: PyObjectRef, value: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        let mut elements = self.elements.borrow_mut();

        if objtype::isinstance(&key, &vm.ctx.int_type()) {
            let idx = objint::get_value(&key).to_i32().unwrap();
            if let Some(pos_index) = elements.get_pos(idx) {
                elements[pos_index] = value;
                Ok(vm.get_none())
            } else {
                Err(vm.new_index_error("list index out of range".to_string()))
            }
        } else {
            panic!(
                "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
                elements, key
            )
        }
    }

    fn repr(self, vm: &mut VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            let mut str_parts = vec![];
            for elem in self.elements.borrow().iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(objstr::get_value(&s));
            }
            format!("[{}]", str_parts.join(", "))
        } else {
            "[...]".to_string()
        };
        Ok(s)
    }

    fn mul(self, counter: isize, vm: &mut VirtualMachine) -> PyObjectRef {
        let new_elements = seq_mul(&self.elements.borrow(), counter);
        vm.ctx.new_list(new_elements)
    }

    fn count(self, needle: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.elements.borrow().iter() {
            if needle.is(element) {
                count += 1;
            } else {
                let py_equal = vm._eq(element.clone(), needle.clone())?;
                if objbool::boolval(vm, py_equal)? {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    fn contains(self, needle: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        for element in self.elements.borrow().iter() {
            if needle.is(element) {
                return Ok(true);
            }
            let py_equal = vm._eq(element.clone(), needle.clone())?;
            if objbool::boolval(vm, py_equal)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn index(self, needle: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<usize> {
        for (index, element) in self.elements.borrow().iter().enumerate() {
            if needle.is(element) {
                return Ok(index);
            }
            let py_equal = vm._eq(needle.clone(), element.clone())?;
            if objbool::boolval(vm, py_equal)? {
                return Ok(index);
            }
        }
        let needle_str = objstr::get_value(&vm.to_str(&needle).unwrap());
        Err(vm.new_value_error(format!("'{}' is not in list", needle_str)))
    }

    fn pop(self, i: OptionalArg<isize>, vm: &mut VirtualMachine) -> PyResult {
        let mut i = i.into_option().unwrap_or(-1);
        let mut elements = self.elements.borrow_mut();
        if i < 0 {
            i += elements.len() as isize;
        }
        if elements.is_empty() {
            Err(vm.new_index_error("pop from empty list".to_string()))
        } else if i < 0 || i as usize >= elements.len() {
            Err(vm.new_index_error("pop index out of range".to_string()))
        } else {
            Ok(elements.remove(i as usize))
        }
    }

    fn remove(self, needle: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<()> {
        let mut ri: Option<usize> = None;
        for (index, element) in self.elements.borrow().iter().enumerate() {
            if needle.is(element) {
                ri = Some(index);
                break;
            }
            let py_equal = vm._eq(needle.clone(), element.clone())?;
            if objbool::get_value(&py_equal) {
                ri = Some(index);
                break;
            }
        }

        if let Some(index) = ri {
            self.elements.borrow_mut().remove(index);
            Ok(())
        } else {
            let needle_str = objstr::get_value(&vm.to_str(&needle)?);
            Err(vm.new_value_error(format!("'{}' is not in list", needle_str)))
        }
    }

    fn eq(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if self.as_object().is(&other) {
            return Ok(true);
        }

        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            Ok(seq_equal(vm, &zelf, &other)?)
        } else {
            Ok(false)
        }
    }

    fn lt(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            Ok(seq_lt(vm, &zelf, &other)?)
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {} using '<'", self, other)))
        }
    }

    fn gt(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            Ok(seq_gt(vm, &zelf, &other)?)
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {} using '>'", self, other)))
        }
    }

    fn ge(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            Ok(seq_ge(vm, &zelf, &other)?)
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {} using '>='", self, other)))
        }
    }

    fn le(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            Ok(seq_le(vm, &zelf, &other)?)
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {} using '<='", self, other)))
        }
    }
}

fn list_new(
    cls: PyRef<objtype::PyClass>,
    iterable: OptionalArg<PyObjectRef>,
    vm: &mut VirtualMachine,
) -> PyResult {
    if !objtype::issubclass(cls.as_object(), &vm.ctx.list_type()) {
        return Err(vm.new_type_error(format!("{} is not a subtype of list", cls)));
    }

    let elements = if let OptionalArg::Present(iterable) = iterable {
        vm.extract_elements(&iterable)?
    } else {
        vec![]
    };

    Ok(PyObject::new(PyList::from(elements), cls.into_object()))
}

fn quicksort(
    vm: &mut VirtualMachine,
    keys: &mut [PyObjectRef],
    values: &mut [PyObjectRef],
) -> PyResult<()> {
    let len = values.len();
    if len >= 2 {
        let pivot = partition(vm, keys, values)?;
        quicksort(vm, &mut keys[0..pivot], &mut values[0..pivot])?;
        quicksort(vm, &mut keys[pivot + 1..len], &mut values[pivot + 1..len])?;
    }
    Ok(())
}

fn partition(
    vm: &mut VirtualMachine,
    keys: &mut [PyObjectRef],
    values: &mut [PyObjectRef],
) -> PyResult<usize> {
    let len = values.len();
    let pivot = len / 2;

    values.swap(pivot, len - 1);
    keys.swap(pivot, len - 1);

    let mut store_idx = 0;
    for i in 0..len - 1 {
        let result = vm._lt(keys[i].clone(), keys[len - 1].clone())?;
        let boolval = objbool::boolval(vm, result)?;
        if boolval {
            values.swap(i, store_idx);
            keys.swap(i, store_idx);
            store_idx += 1;
        }
    }

    values.swap(store_idx, len - 1);
    keys.swap(store_idx, len - 1);
    Ok(store_idx)
}

fn do_sort(
    vm: &mut VirtualMachine,
    values: &mut Vec<PyObjectRef>,
    key_func: Option<PyObjectRef>,
    reverse: bool,
) -> PyResult<()> {
    // build a list of keys. If no keyfunc is provided, it's a copy of the list.
    let mut keys: Vec<PyObjectRef> = vec![];
    for x in values.iter() {
        keys.push(match &key_func {
            None => x.clone(),
            Some(ref func) => vm.invoke((*func).clone(), vec![x.clone()])?,
        });
    }

    quicksort(vm, &mut keys, values)?;

    if reverse {
        values.reverse();
    }

    Ok(())
}

fn list_sort(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let key_func = args.get_optional_kwarg("key");
    let reverse = args.get_optional_kwarg("reverse");
    let reverse = match reverse {
        None => false,
        Some(val) => objbool::boolval(vm, val)?,
    };

    let elements_cell = get_elements_cell(list);
    // replace list contents with [] for duration of sort.
    // this prevents keyfunc from messing with the list and makes it easy to
    // check if it tries to append elements to it.
    let mut elements = elements_cell.replace(vec![]);
    do_sort(vm, &mut elements, key_func, reverse)?;
    let temp_elements = elements_cell.replace(elements);

    if !temp_elements.is_empty() {
        return Err(vm.new_value_error("list modified during sort".to_string()));
    }

    Ok(vm.get_none())
}

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &PyContext) {
    let list_type = &context.list_type;

    let list_doc = "Built-in mutable sequence.\n\n\
                    If no argument is given, the constructor creates a new empty list.\n\
                    The argument must be an iterable if specified.";

    context.set_attr(&list_type, "__add__", context.new_rustfunc(PyListRef::add));
    context.set_attr(&list_type, "__iadd__", context.new_rustfunc(PyListRef::iadd));
    context.set_attr(&list_type, "__contains__", context.new_rustfunc(PyListRef::contains));
    context.set_attr(&list_type, "__eq__", context.new_rustfunc(PyListRef::eq));
    context.set_attr(&list_type, "__lt__", context.new_rustfunc(PyListRef::lt));
    context.set_attr(&list_type, "__gt__", context.new_rustfunc(PyListRef::gt));
    context.set_attr(&list_type, "__le__", context.new_rustfunc(PyListRef::le));
    context.set_attr(&list_type, "__ge__", context.new_rustfunc(PyListRef::ge));
    context.set_attr(&list_type, "__getitem__", context.new_rustfunc(PyListRef::getitem));
    context.set_attr(&list_type, "__iter__", context.new_rustfunc(PyListRef::iter));
    context.set_attr(&list_type, "__setitem__", context.new_rustfunc(PyListRef::setitem));
    context.set_attr(&list_type, "__mul__", context.new_rustfunc(PyListRef::mul));
    context.set_attr(&list_type, "__len__", context.new_rustfunc(PyListRef::len));
    context.set_attr(&list_type, "__new__", context.new_rustfunc(list_new));
    context.set_attr(&list_type, crate::VM_REPR, context.new_rustfunc(PyListRef::repr));
    context.set_attr(&list_type, "__doc__", context.new_str(list_doc.to_string()));
    context.set_attr(&list_type, "append", context.new_rustfunc(PyListRef::append));
    context.set_attr(&list_type, "clear", context.new_rustfunc(PyListRef::clear));
    context.set_attr(&list_type, "copy", context.new_rustfunc(PyListRef::copy));
    context.set_attr(&list_type, "count", context.new_rustfunc(PyListRef::count));
    context.set_attr(&list_type, "extend", context.new_rustfunc(PyListRef::extend));
    context.set_attr(&list_type, "index", context.new_rustfunc(PyListRef::index));
    context.set_attr(&list_type, "insert", context.new_rustfunc(PyListRef::insert));
    context.set_attr(&list_type, "reverse", context.new_rustfunc(PyListRef::reverse));
    context.set_attr(&list_type, "sort", context.new_rustfunc(list_sort));
    context.set_attr(&list_type, "pop", context.new_rustfunc(PyListRef::pop));
    context.set_attr(&list_type, "remove", context.new_rustfunc(PyListRef::remove));
}
