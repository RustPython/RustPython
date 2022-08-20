use rustpython_common::lock::PyRwLock;

use crate::types::{Constructor, GetDescriptor, Unconstructible};
use crate::{Context, Py, PyObjectRef, PyRef, PyResult, VirtualMachine};

use super::{PyStr, PyType, PyTypeRef};
use crate::class::PyClassImpl;
use crate::object::PyPayload;

#[derive(Debug)]
pub struct DescrObject {
    pub typ: PyTypeRef,
    pub name: String,
    pub qualname: PyRwLock<Option<String>>,
}

#[derive(Debug)]
pub enum MemberKind {
    ObjectEx = 16,
}

pub struct MemberDef {
    pub name: String,
    pub kind: MemberKind,
    pub getter: fn(PyObjectRef, &VirtualMachine) -> PyResult,
    pub doc: Option<String>,
}

impl std::fmt::Debug for MemberDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberDef")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("doc", &self.doc)
            .finish()
    }
}

#[pyclass(name = "member_descriptor", module = false)]
#[derive(Debug)]
pub struct MemberDescrObject {
    pub common: DescrObject,
    pub member: MemberDef,
}

impl PyPayload for MemberDescrObject {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.member_descriptor_type
    }
}

fn calculate_qualname(descr: &DescrObject, vm: &VirtualMachine) -> PyResult<Option<String>> {
    let type_qualname = vm.get_attribute_opt(descr.typ.to_owned().into(), "__qualname__")?;
    match type_qualname {
        None => Ok(None),
        Some(obj) => match obj.downcast::<PyStr>() {
            Ok(str) => Ok(Some(format!("{}.{}", str, descr.name))),
            Err(_) => Err(vm.new_type_error(
                "<descriptor>.__objclass__.__qualname__ is not a unicode object".to_owned(),
            )),
        },
    }
}

#[pyclass(with(GetDescriptor, Constructor), flags(BASETYPE))]
impl MemberDescrObject {
    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>) -> String {
        format!(
            "<member '{}' of '{}' objects>",
            zelf.common.name,
            zelf.common.typ.name(),
        )
    }

    #[pyproperty(magic)]
    fn doc(zelf: PyRef<Self>) -> Option<String> {
        zelf.member.doc.to_owned()
    }

    #[pyproperty(magic)]
    fn qualname(&self, vm: &VirtualMachine) -> PyResult<Option<String>> {
        if self.common.qualname.read().is_none() {
            *self.common.qualname.write() = calculate_qualname(&self.common, vm)?;
        }

        Ok(self.common.qualname.read().to_owned())
    }
}

impl Unconstructible for MemberDescrObject {}

impl GetDescriptor for MemberDescrObject {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Ok(match obj {
            Some(x) => {
                let zelf = Self::_zelf(zelf, vm)?;
                (zelf.member.getter)(x, vm)?
            }
            None => zelf,
        })
    }
}

pub fn init(context: &Context) {
    let member_descriptor_type = &context.types.member_descriptor_type;
    MemberDescrObject::extend_class(context, member_descriptor_type);
}
