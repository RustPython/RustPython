use crate::bytecode::*;
use crate::marshal::{self, Read, ReadBorrowed, Write};

/// A frozen module. Holds a frozen code object and whether it is part of a package
#[derive(Copy, Clone)]
pub struct FrozenModule<B = &'static [u8]> {
    pub code: FrozenCodeObject<B>,
    pub package: bool,
}

#[derive(Copy, Clone)]
pub struct FrozenCodeObject<B> {
    pub bytes: B,
}

impl<B: AsRef<[u8]>> FrozenCodeObject<B> {
    /// Decode a frozen code object
    #[inline]
    pub fn decode<Bag: AsBag>(&self, bag: Bag) -> CodeObject<<Bag::Bag as ConstantBag>::Constant> {
        Self::_decode(self.bytes.as_ref(), bag.as_bag())
    }
    fn _decode<Bag: ConstantBag>(data: &[u8], bag: Bag) -> CodeObject<Bag::Constant> {
        let decompressed = lz4_flex::decompress_size_prepended(data)
            .expect("deserialize frozen CodeObject failed");
        marshal::deserialize_code(&mut &decompressed[..], bag)
            .expect("deserializing frozen CodeObject failed")
    }
}

impl FrozenCodeObject<Vec<u8>> {
    pub fn encode<C: Constant>(code: &CodeObject<C>) -> Self {
        let mut data = Vec::new();
        marshal::serialize_code(&mut data, code);
        let bytes = lz4_flex::compress_prepend_size(&data);
        Self { bytes }
    }
}

#[repr(transparent)]
pub struct FrozenLib<B: ?Sized = [u8]> {
    pub bytes: B,
}

impl<B: AsRef<[u8]> + ?Sized> FrozenLib<B> {
    pub const fn from_ref(b: &B) -> &Self {
        unsafe { &*(b as *const B as *const Self) }
    }

    /// Decode a library to a iterable of frozen modules
    pub fn decode(&self) -> FrozenModulesIter<'_> {
        let mut data = self.bytes.as_ref();
        let remaining = data.read_u32().unwrap();
        FrozenModulesIter { remaining, data }
    }
}

impl<'a, B: AsRef<[u8]> + ?Sized> IntoIterator for &'a FrozenLib<B> {
    type Item = (&'a str, FrozenModule<&'a [u8]>);
    type IntoIter = FrozenModulesIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.decode()
    }
}

pub struct FrozenModulesIter<'a> {
    remaining: u32,
    data: &'a [u8],
}

impl<'a> Iterator for FrozenModulesIter<'a> {
    type Item = (&'a str, FrozenModule<&'a [u8]>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining > 0 {
            let entry = read_entry(&mut self.data).unwrap();
            self.remaining -= 1;
            Some(entry)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining as usize, Some(self.remaining as usize))
    }
}

impl ExactSizeIterator for FrozenModulesIter<'_> {}

fn read_entry<'a>(
    rdr: &mut &'a [u8],
) -> Result<(&'a str, FrozenModule<&'a [u8]>), marshal::MarshalError> {
    let len = rdr.read_u32()?;
    let name = rdr.read_str_borrow(len)?;
    let len = rdr.read_u32()?;
    let code_slice = rdr.read_slice_borrow(len)?;
    let code = FrozenCodeObject { bytes: code_slice };
    let package = rdr.read_u8()? != 0;
    Ok((name, FrozenModule { code, package }))
}

impl FrozenLib<Vec<u8>> {
    /// Encode the given iterator of frozen modules into a compressed vector of bytes
    pub fn encode<'a, I, B: AsRef<[u8]>>(lib: I) -> Self
    where
        I: IntoIterator<Item = (&'a str, FrozenModule<B>), IntoIter: ExactSizeIterator + Clone>,
    {
        let iter = lib.into_iter();
        let mut bytes = Vec::new();
        write_lib(&mut bytes, iter);
        Self { bytes }
    }
}

fn write_lib<'a, B: AsRef<[u8]>>(
    buf: &mut Vec<u8>,
    lib: impl ExactSizeIterator<Item = (&'a str, FrozenModule<B>)>,
) {
    marshal::write_len(buf, lib.len());
    for (name, module) in lib {
        write_entry(buf, name, module);
    }
}

fn write_entry(buf: &mut Vec<u8>, name: &str, module: FrozenModule<impl AsRef<[u8]>>) {
    marshal::write_vec(buf, name.as_bytes());
    marshal::write_vec(buf, module.code.bytes.as_ref());
    buf.write_u8(module.package as u8);
}
