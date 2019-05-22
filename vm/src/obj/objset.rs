/*
 * Builtin set type with a sequence of unique items.
 */

use std::cell::{Cell, RefCell};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::function::OptionalArg;
use crate::pyobject::{
    PyContext, PyIterable, PyObject, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::vm::{ReprGuard, VirtualMachine};

use super::objbool;
use super::objint;
use super::objlist::PyListIterator;
use super::objtype;
use super::objtype::PyClassRef;

#[derive(Default)]
pub struct PySet {
    inner: RefCell<PySetInner>,
}
pub type PySetRef = PyRef<PySet>;

#[derive(Default)]
pub struct PyFrozenSet {
    inner: PySetInner,
}
pub type PyFrozenSetRef = PyRef<PyFrozenSet>;

impl fmt::Debug for PySet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("set")
    }
}

impl fmt::Debug for PyFrozenSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("frozenset")
    }
}

impl PyValue for PySet {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.set_type()
    }
}

impl PyValue for PyFrozenSet {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.frozenset_type()
    }
}

#[derive(Default, Clone)]
struct PySetInner {
    elements: HashMap<u64, PyObjectRef>,
}

impl PySetInner {
    fn new(iterable: OptionalArg<PyIterable>, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let elements: HashMap<u64, PyObjectRef> = match iterable {
            OptionalArg::Missing => HashMap::new(),
            OptionalArg::Present(iterable) => {
                let mut elements = HashMap::new();
                for item in iterable.iter(vm)? {
                    insert_into_set(vm, &mut elements, &item?)?;
                }
                elements
            }
        };

        Ok(PySetInner { elements })
    }

    fn len(&self) -> usize {
        self.elements.len()
    }
    fn copy(&self) -> PySetInner {
        PySetInner {
            elements: self.elements.clone(),
        }
    }

    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.elements.iter() {
            let value = vm._eq(needle.clone(), element.1.clone())?;
            if objbool::get_value(&value) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn _compare_inner(
        &self,
        other: &PySetInner,
        size_func: &Fn(usize, usize) -> bool,
        swap: bool,
        vm: &VirtualMachine,
    ) -> PyResult {
        let get_zelf = |swap: bool| -> &PySetInner {
            if swap {
                other
            } else {
                self
            }
        };
        let get_other = |swap: bool| -> &PySetInner {
            if swap {
                self
            } else {
                other
            }
        };

        if size_func(get_zelf(swap).len(), get_other(swap).len()) {
            return Ok(vm.new_bool(false));
        }
        for element in get_other(swap).elements.iter() {
            if !get_zelf(swap).contains(element.1.clone(), vm)? {
                return Ok(vm.new_bool(false));
            }
        }
        Ok(vm.new_bool(true))
    }

    fn eq(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult {
        self._compare_inner(
            other,
            &|zelf: usize, other: usize| -> bool { zelf != other },
            false,
            vm,
        )
    }

    fn ge(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult {
        self._compare_inner(
            other,
            &|zelf: usize, other: usize| -> bool { zelf < other },
            false,
            vm,
        )
    }

    fn gt(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult {
        self._compare_inner(
            other,
            &|zelf: usize, other: usize| -> bool { zelf <= other },
            false,
            vm,
        )
    }

    fn le(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult {
        self._compare_inner(
            other,
            &|zelf: usize, other: usize| -> bool { zelf < other },
            true,
            vm,
        )
    }

    fn lt(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult {
        self._compare_inner(
            other,
            &|zelf: usize, other: usize| -> bool { zelf <= other },
            true,
            vm,
        )
    }

    fn union(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let mut elements = self.elements.clone();
        for item in other.iter(vm)? {
            insert_into_set(vm, &mut elements, &item?)?;
        }

        Ok(PySetInner { elements })
    }

    fn intersection(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let mut elements = HashMap::new();
        for item in other.iter(vm)? {
            let obj = item?;
            if self.contains(obj.clone(), vm)? {
                insert_into_set(vm, &mut elements, &obj)?;
            }
        }
        Ok(PySetInner { elements })
    }

    fn difference(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let mut elements = self.elements.clone();
        for item in other.iter(vm)? {
            let obj = item?;
            if self.contains(obj.clone(), vm)? {
                remove_from_set(vm, &mut elements, &obj)?;
            }
        }
        Ok(PySetInner { elements })
    }

    fn symmetric_difference(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let mut new_inner = self.clone();

        for item in other.iter(vm)? {
            let obj = item?;
            if !self.contains(obj.clone(), vm)? {
                new_inner.add(&obj, vm)?;
            } else {
                new_inner.remove(&obj, vm)?;
            }
        }

        Ok(new_inner)
    }

    fn isdisjoint(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        for item in other.iter(vm)? {
            let obj = item?;
            if self.contains(obj.clone(), vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn iter(&self, vm: &VirtualMachine) -> PyListIterator {
        let items = self.elements.values().cloned().collect();
        let set_list = vm.ctx.new_list(items);
        PyListIterator {
            position: Cell::new(0),
            list: set_list.downcast().unwrap(),
        }
    }

    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        let mut str_parts = vec![];
        for elem in self.elements.values() {
            let part = vm.to_repr(elem)?;
            str_parts.push(part.value.clone());
        }

        Ok(format!("{{{}}}", str_parts.join(", ")))
    }

    fn add(&mut self, item: &PyObjectRef, vm: &VirtualMachine) -> PyResult {
        insert_into_set(vm, &mut self.elements, &item)
    }

    fn remove(&mut self, item: &PyObjectRef, vm: &VirtualMachine) -> PyResult {
        remove_from_set(vm, &mut self.elements, &item)
    }

    fn discard(&mut self, item: &PyObjectRef, vm: &VirtualMachine) -> PyResult {
        fn discard(
            vm: &VirtualMachine,
            elements: &mut HashMap<u64, PyObjectRef>,
            key: u64,
            _value: &PyObjectRef,
        ) -> PyResult {
            elements.remove(&key);
            Ok(vm.get_none())
        }
        perform_action_with_hash(vm, &mut self.elements, &item, &discard)
    }

    fn clear(&mut self) {
        self.elements.clear();
    }

    fn pop(&mut self, vm: &VirtualMachine) -> PyResult {
        let elements = &mut self.elements;
        match elements.clone().keys().next() {
            Some(key) => Ok(elements.remove(key).unwrap()),
            None => Err(vm.new_key_error("pop from an empty set".to_string())),
        }
    }

    fn update(&mut self, iterable: PyIterable, vm: &VirtualMachine) -> PyResult {
        for item in iterable.iter(vm)? {
            insert_into_set(vm, &mut self.elements, &item?)?;
        }
        Ok(vm.get_none())
    }

    fn intersection_update(&mut self, iterable: PyIterable, vm: &VirtualMachine) -> PyResult {
        let temp_inner = self.copy();
        self.clear();
        for item in iterable.iter(vm)? {
            let obj = item?;
            if temp_inner.contains(obj.clone(), vm)? {
                self.add(&obj, vm)?;
            }
        }
        Ok(vm.get_none())
    }

    fn difference_update(&mut self, iterable: PyIterable, vm: &VirtualMachine) -> PyResult {
        for item in iterable.iter(vm)? {
            let obj = item?;
            if self.contains(obj.clone(), vm)? {
                self.remove(&obj, vm)?;
            }
        }
        Ok(vm.get_none())
    }

    fn symmetric_difference_update(
        &mut self,
        iterable: PyIterable,
        vm: &VirtualMachine,
    ) -> PyResult {
        for item in iterable.iter(vm)? {
            let obj = item?;
            if !self.contains(obj.clone(), vm)? {
                self.add(&obj, vm)?;
            } else {
                self.remove(&obj, vm)?;
            }
        }
        Ok(vm.get_none())
    }
}

impl PySetRef {
    fn new(
        cls: PyClassRef,
        iterable: OptionalArg<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<PySetRef> {
        PySet {
            inner: RefCell::new(PySetInner::new(iterable, vm)?),
        }
        .into_ref_with_type(vm, cls)
    }

    fn len(self, _vm: &VirtualMachine) -> usize {
        self.inner.borrow().len()
    }

    fn copy(self, _vm: &VirtualMachine) -> PySet {
        PySet {
            inner: RefCell::new(self.inner.borrow().copy()),
        }
    }

    fn contains(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.borrow().contains(needle, vm)
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.borrow().eq(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.borrow().eq(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.borrow().ge(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.borrow().ge(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.borrow().gt(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.borrow().gt(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.borrow().le(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.borrow().le(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.borrow().lt(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.borrow().lt(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn union(self, other: PyIterable, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(
            PySet {
                inner: RefCell::new(self.inner.borrow().union(other, vm)?),
            },
            PySet::class(vm),
            None,
        ))
    }

    fn intersection(self, other: PyIterable, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(
            PySet {
                inner: RefCell::new(self.inner.borrow().intersection(other, vm)?),
            },
            PySet::class(vm),
            None,
        ))
    }

    fn difference(self, other: PyIterable, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(
            PySet {
                inner: RefCell::new(self.inner.borrow().difference(other, vm)?),
            },
            PySet::class(vm),
            None,
        ))
    }

    fn symmetric_difference(self, other: PyIterable, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(
            PySet {
                inner: RefCell::new(self.inner.borrow().symmetric_difference(other, vm)?),
            },
            PySet::class(vm),
            None,
        ))
    }

    fn isdisjoint(self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.borrow().isdisjoint(other, vm)
    }

    fn or(self, other: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.union(other.iterable, vm)
    }

    fn and(self, other: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.intersection(other.iterable, vm)
    }

    fn sub(self, other: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.difference(other.iterable, vm)
    }

    fn xor(self, other: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.symmetric_difference(other.iterable, vm)
    }

    fn iter(self, vm: &VirtualMachine) -> PyListIterator {
        self.inner.borrow().iter(vm)
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult {
        let inner = self.inner.borrow();
        let s = if inner.len() == 0 {
            "set()".to_string()
        } else if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            inner.repr(vm)?
        } else {
            "set(...)".to_string()
        };
        Ok(vm.new_str(s))
    }

    pub fn add(self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow_mut().add(&item, vm)
    }

    fn remove(self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow_mut().remove(&item, vm)
    }

    fn discard(self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow_mut().discard(&item, vm)
    }

    fn clear(self, _vm: &VirtualMachine) {
        self.inner.borrow_mut().clear()
    }

    fn pop(self, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow_mut().pop(vm)
    }

    fn ior(self, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow_mut().update(iterable.iterable, vm)?;
        Ok(self.as_object().clone())
    }

    fn update(self, iterable: PyIterable, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow_mut().update(iterable, vm)?;
        Ok(vm.get_none())
    }

    fn intersection_update(self, iterable: PyIterable, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow_mut().intersection_update(iterable, vm)?;
        Ok(vm.get_none())
    }

    fn iand(self, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.inner
            .borrow_mut()
            .intersection_update(iterable.iterable, vm)?;
        Ok(self.as_object().clone())
    }

    fn difference_update(self, iterable: PyIterable, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow_mut().difference_update(iterable, vm)?;
        Ok(vm.get_none())
    }

    fn isub(self, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.inner
            .borrow_mut()
            .difference_update(iterable.iterable, vm)?;
        Ok(self.as_object().clone())
    }

    fn symmetric_difference_update(self, iterable: PyIterable, vm: &VirtualMachine) -> PyResult {
        self.inner
            .borrow_mut()
            .symmetric_difference_update(iterable, vm)?;
        Ok(vm.get_none())
    }

    fn ixor(self, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.inner
            .borrow_mut()
            .symmetric_difference_update(iterable.iterable, vm)?;
        Ok(self.as_object().clone())
    }
}

impl PyFrozenSetRef {
    fn new(
        cls: PyClassRef,
        iterable: OptionalArg<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<PyFrozenSetRef> {
        PyFrozenSet {
            inner: PySetInner::new(iterable, vm)?,
        }
        .into_ref_with_type(vm, cls)
    }

    fn len(self, _vm: &VirtualMachine) -> usize {
        self.inner.len()
    }

    fn copy(self, _vm: &VirtualMachine) -> PyFrozenSet {
        PyFrozenSet {
            inner: self.inner.copy(),
        }
    }

    fn contains(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.contains(needle, vm)
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.eq(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.eq(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.ge(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.ge(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.gt(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.gt(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.le(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.le(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        set @ PySet => self.inner.lt(&set.inner.borrow(), vm),
        frozen @ PyFrozenSet => self.inner.lt(&frozen.inner, vm),
        other =>  {return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", other.class())));},
        )
    }

    fn union(self, other: PyIterable, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(
            PyFrozenSet {
                inner: self.inner.union(other, vm)?,
            },
            PyFrozenSet::class(vm),
            None,
        ))
    }

    fn intersection(self, other: PyIterable, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(
            PyFrozenSet {
                inner: self.inner.intersection(other, vm)?,
            },
            PyFrozenSet::class(vm),
            None,
        ))
    }

    fn difference(self, other: PyIterable, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(
            PyFrozenSet {
                inner: self.inner.difference(other, vm)?,
            },
            PyFrozenSet::class(vm),
            None,
        ))
    }

    fn symmetric_difference(self, other: PyIterable, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(
            PyFrozenSet {
                inner: self.inner.symmetric_difference(other, vm)?,
            },
            PyFrozenSet::class(vm),
            None,
        ))
    }

    fn isdisjoint(self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.isdisjoint(other, vm)
    }

    fn or(self, other: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.union(other.iterable, vm)
    }

    fn and(self, other: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.intersection(other.iterable, vm)
    }

    fn sub(self, other: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.difference(other.iterable, vm)
    }

    fn xor(self, other: SetIterable, vm: &VirtualMachine) -> PyResult {
        self.symmetric_difference(other.iterable, vm)
    }

    fn iter(self, vm: &VirtualMachine) -> PyListIterator {
        self.inner.iter(vm)
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult {
        let inner = &self.inner;
        let s = if inner.len() == 0 {
            "frozenset()".to_string()
        } else if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            format!("frozenset({})", inner.repr(vm)?)
        } else {
            "frozenset(...)".to_string()
        };
        Ok(vm.new_str(s))
    }
}

fn perform_action_with_hash(
    vm: &VirtualMachine,
    elements: &mut HashMap<u64, PyObjectRef>,
    item: &PyObjectRef,
    f: &Fn(&VirtualMachine, &mut HashMap<u64, PyObjectRef>, u64, &PyObjectRef) -> PyResult,
) -> PyResult {
    let hash: PyObjectRef = vm.call_method(item, "__hash__", vec![])?;

    let hash_value = objint::get_value(&hash);
    let mut hasher = DefaultHasher::new();
    hash_value.hash(&mut hasher);
    let key = hasher.finish();
    f(vm, elements, key, item)
}

fn insert_into_set(
    vm: &VirtualMachine,
    elements: &mut HashMap<u64, PyObjectRef>,
    item: &PyObjectRef,
) -> PyResult {
    fn insert(
        vm: &VirtualMachine,
        elements: &mut HashMap<u64, PyObjectRef>,
        key: u64,
        value: &PyObjectRef,
    ) -> PyResult {
        elements.insert(key, value.clone());
        Ok(vm.get_none())
    }
    perform_action_with_hash(vm, elements, item, &insert)
}

fn remove_from_set(
    vm: &VirtualMachine,
    elements: &mut HashMap<u64, PyObjectRef>,
    item: &PyObjectRef,
) -> PyResult {
    fn remove(
        vm: &VirtualMachine,
        elements: &mut HashMap<u64, PyObjectRef>,
        key: u64,
        value: &PyObjectRef,
    ) -> PyResult {
        match elements.remove(&key) {
            None => {
                let item_str = format!("{:?}", value);
                Err(vm.new_key_error(item_str))
            }
            Some(_) => Ok(vm.get_none()),
        }
    }
    perform_action_with_hash(vm, elements, item, &remove)
}

struct SetIterable {
    iterable: PyIterable,
}

impl TryFromObject for SetIterable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if objtype::issubclass(&obj.class(), &vm.ctx.set_type())
            || objtype::issubclass(&obj.class(), &vm.ctx.frozenset_type())
        {
            Ok(SetIterable {
                iterable: PyIterable::try_from_object(vm, obj)?,
            })
        } else {
            Err(vm.new_type_error(format!(
                "{} is not a subtype of set or frozenset",
                obj.class()
            )))
        }
    }
}

fn set_hash(_zelf: PySetRef, vm: &VirtualMachine) -> PyResult<()> {
    Err(vm.new_type_error("unhashable type".to_string()))
}

pub fn init(context: &PyContext) {
    let set_type = &context.set_type;

    let set_doc = "set() -> new empty set object\n\
                   set(iterable) -> new set object\n\n\
                   Build an unordered collection of unique elements.";

    extend_class!(context, set_type, {
        "__hash__" => context.new_rustfunc(set_hash),
        "__contains__" => context.new_rustfunc(PySetRef::contains),
        "__len__" => context.new_rustfunc(PySetRef::len),
        "__new__" => context.new_rustfunc(PySetRef::new),
        "__repr__" => context.new_rustfunc(PySetRef::repr),
        "__eq__" => context.new_rustfunc(PySetRef::eq),
        "__ge__" => context.new_rustfunc(PySetRef::ge),
        "__gt__" => context.new_rustfunc(PySetRef::gt),
        "__le__" => context.new_rustfunc(PySetRef::le),
        "__lt__" => context.new_rustfunc(PySetRef::lt),
        "issubset" => context.new_rustfunc(PySetRef::le),
        "issuperset" => context.new_rustfunc(PySetRef::ge),
        "union" => context.new_rustfunc(PySetRef::union),
        "__or__" => context.new_rustfunc(PySetRef::or),
        "intersection" => context.new_rustfunc(PySetRef::intersection),
        "__and__" => context.new_rustfunc(PySetRef::and),
        "difference" => context.new_rustfunc(PySetRef::difference),
        "__sub__" => context.new_rustfunc(PySetRef::sub),
        "symmetric_difference" => context.new_rustfunc(PySetRef::symmetric_difference),
        "__xor__" => context.new_rustfunc(PySetRef::xor),
        "__doc__" => context.new_str(set_doc.to_string()),
        "add" => context.new_rustfunc(PySetRef::add),
        "remove" => context.new_rustfunc(PySetRef::remove),
        "discard" => context.new_rustfunc(PySetRef::discard),
        "clear" => context.new_rustfunc(PySetRef::clear),
        "copy" => context.new_rustfunc(PySetRef::copy),
        "pop" => context.new_rustfunc(PySetRef::pop),
        "update" => context.new_rustfunc(PySetRef::update),
        "__ior__" => context.new_rustfunc(PySetRef::ior),
        "intersection_update" => context.new_rustfunc(PySetRef::intersection_update),
        "__iand__" => context.new_rustfunc(PySetRef::iand),
        "difference_update" => context.new_rustfunc(PySetRef::difference_update),
        "__isub__" => context.new_rustfunc(PySetRef::isub),
        "symmetric_difference_update" => context.new_rustfunc(PySetRef::symmetric_difference_update),
        "__ixor__" => context.new_rustfunc(PySetRef::ixor),
        "__iter__" => context.new_rustfunc(PySetRef::iter),
        "isdisjoint" => context.new_rustfunc(PySetRef::isdisjoint),
    });

    let frozenset_type = &context.frozenset_type;

    let frozenset_doc = "frozenset() -> empty frozenset object\n\
                         frozenset(iterable) -> frozenset object\n\n\
                         Build an immutable unordered collection of unique elements.";

    extend_class!(context, frozenset_type, {
        "__new__" => context.new_rustfunc(PyFrozenSetRef::new),
        "__eq__" => context.new_rustfunc(PyFrozenSetRef::eq),
        "__ge__" => context.new_rustfunc(PyFrozenSetRef::ge),
        "__gt__" => context.new_rustfunc(PyFrozenSetRef::gt),
        "__le__" => context.new_rustfunc(PyFrozenSetRef::le),
        "__lt__" => context.new_rustfunc(PyFrozenSetRef::lt),
        "issubset" => context.new_rustfunc(PyFrozenSetRef::le),
        "issuperset" => context.new_rustfunc(PyFrozenSetRef::ge),
        "union" => context.new_rustfunc(PyFrozenSetRef::union),
        "__or__" => context.new_rustfunc(PyFrozenSetRef::or),
        "intersection" => context.new_rustfunc(PyFrozenSetRef::intersection),
        "__and__" => context.new_rustfunc(PyFrozenSetRef::and),
        "difference" => context.new_rustfunc(PyFrozenSetRef::difference),
        "__sub__" => context.new_rustfunc(PyFrozenSetRef::sub),
        "symmetric_difference" => context.new_rustfunc(PyFrozenSetRef::symmetric_difference),
        "__xor__" => context.new_rustfunc(PyFrozenSetRef::xor),
        "__contains__" => context.new_rustfunc(PyFrozenSetRef::contains),
        "__len__" => context.new_rustfunc(PyFrozenSetRef::len),
        "__doc__" => context.new_str(frozenset_doc.to_string()),
        "__repr__" => context.new_rustfunc(PyFrozenSetRef::repr),
        "copy" => context.new_rustfunc(PyFrozenSetRef::copy),
        "__iter__" => context.new_rustfunc(PyFrozenSetRef::iter),
        "isdisjoint" => context.new_rustfunc(PyFrozenSetRef::isdisjoint),
    });
}
