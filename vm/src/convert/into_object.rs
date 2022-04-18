use crate::PyObjectRef;

pub trait IntoObject
where
    Self: Into<PyObjectRef>,
{
    fn into_object(self) -> PyObjectRef {
        self.into()
    }
}

impl<T> IntoObject for T where T: Into<PyObjectRef> {}
