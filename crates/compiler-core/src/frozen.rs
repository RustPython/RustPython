use crate::bytecode::*;
use crate::marshal::{self, Read, ReadBorrowed, Write};
use alloc::vec::Vec;

/// A frozen module. Holds a frozen code object and whether it is part of a package
#[derive(Copy, Clone)]
pub struct FrozenModule<B = &'static [u8]> {
    pub code: FrozenCodeObject<B>,
    pub package: bool,
    /// Empty package generated for a source namespace directory. Genuine
    /// modules take precedence over these containers across frozen libraries.
    /// When true, `package` must also be true.
    pub namespace_container: bool,
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
    let flags = rdr.read_u8()?;
    Ok((
        name,
        FrozenModule {
            code,
            package: flags & 1 != 0,
            namespace_container: flags & 2 != 0,
        },
    ))
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
    assert!(!module.namespace_container || module.package);
    marshal::write_vec(buf, name.as_bytes());
    marshal::write_vec(buf, module.code.bytes.as_ref());
    // Ordinary modules retain the legacy 0/1 encoding. Older readers treat
    // container flag 3 as a package, but cannot preserve container priority.
    buf.write_u8(module.package as u8 | ((module.namespace_container as u8) << 1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_flags_preserve_legacy_and_container_metadata() {
        // Opaque bytes exercise only the library envelope, not Python bytecode.
        for (package, namespace_container, flags) in
            [(false, false, 0), (true, false, 1), (true, true, 3)]
        {
            let module = FrozenModule {
                code: FrozenCodeObject { bytes: &[7_u8][..] },
                package,
                namespace_container,
            };
            let encoded = FrozenLib::encode([("entry", module)]);
            assert_eq!(encoded.bytes.last(), Some(&flags));
            if !namespace_container {
                let mut legacy = Vec::new();
                marshal::write_len(&mut legacy, 1);
                marshal::write_vec(&mut legacy, b"entry");
                marshal::write_vec(&mut legacy, &[7]);
                legacy.write_u8(package as u8);
                assert_eq!(encoded.bytes, legacy);
            }
            let mut decoded = encoded.decode();
            let (name, decoded_module) = decoded.next().unwrap();
            assert_eq!(name, "entry");
            assert_eq!(decoded_module.code.bytes, &[7]);
            assert_eq!(decoded_module.package, package);
            assert_eq!(decoded_module.namespace_container, namespace_container);
            assert!(decoded.next().is_none());
        }
        let entries = [
            (
                "real",
                FrozenModule {
                    code: FrozenCodeObject {
                        bytes: &[1_u8, 2][..],
                    },
                    package: false,
                    namespace_container: false,
                },
            ),
            (
                "container",
                FrozenModule {
                    code: FrozenCodeObject { bytes: &[3_u8][..] },
                    package: true,
                    namespace_container: true,
                },
            ),
        ];
        let encoded = FrozenLib::encode(entries);
        let decoded: Vec<_> = encoded.decode().collect();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].0, "real");
        assert_eq!(decoded[0].1.code.bytes, &[1, 2]);
        assert!(!decoded[0].1.namespace_container);
        assert_eq!(decoded[1].0, "container");
        assert_eq!(decoded[1].1.code.bytes, &[3]);
        assert!(decoded[1].1.namespace_container);
    }
}
