// spell-checker:ignore chunker

//! VM-independent compression engines shared by RustPython components.

const CHUNKSIZE: usize = u32::MAX as usize;

/// A two-slice cursor used by streaming decompressors to consume buffered and
/// newly supplied input without joining them first.
#[doc(hidden)]
#[derive(Clone)]
pub struct Chunker<'a> {
    data1: &'a [u8],
    data2: &'a [u8],
}

impl<'a> Chunker<'a> {
    /// Start a cursor over one input slice.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self {
            data1: data,
            data2: &[],
        }
    }

    /// Chain previously buffered input in front of newly supplied input.
    #[must_use]
    pub const fn chain(data1: &'a [u8], data2: &'a [u8]) -> Self {
        if data1.is_empty() {
            Self {
                data1: data2,
                data2: &[],
            }
        } else {
            Self { data1, data2 }
        }
    }

    /// Return the number of bytes that have not been consumed.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.data1.len() + self.data2.len()
    }

    /// Return whether all input has been consumed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.data1.is_empty()
    }

    /// Copy the remaining input into a contiguous vector.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        [self.data1, self.data2].concat()
    }

    /// Return the next native-codec-sized input chunk.
    #[must_use]
    pub fn chunk(&self) -> &'a [u8] {
        self.data1.get(..CHUNKSIZE).unwrap_or(self.data1)
    }

    /// Advance the cursor by `consumed` bytes.
    pub fn advance(&mut self, consumed: usize) {
        self.data1 = &self.data1[consumed..];
        if self.data1.is_empty() {
            self.data1 = core::mem::take(&mut self.data2);
        }
    }
}

#[cfg(feature = "bz2")]
pub mod bz2;
#[cfg(all(
    feature = "lzma",
    not(any(target_os = "android", target_arch = "wasm32"))
))]
pub mod lzma;
#[cfg(feature = "zlib")]
pub mod zlib;
