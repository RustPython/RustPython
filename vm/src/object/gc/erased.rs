use std::{
    fmt,
    mem::{size_of, MaybeUninit},
    ptr::{addr_of, addr_of_mut, copy_nonoverlapping, NonNull},
};

#[repr(C)]
#[derive(Clone, Copy)]
/// A pointer for an allocation, extracted out as raw data.
/// This contains both the pointer and all the pointer's metadata, but hidden behind an unknown
/// interpretation.
/// We trust that all pointers (even to `?Sized` or `dyn` types) are 2 words or fewer in size.
/// This is a hack! Like, a big hack!
pub(crate) struct Erased([usize; 2]);

impl Erased {
    /// Construct a new erased pointer to some data from a reference
    ///
    /// # Panics
    ///
    /// This function will panic if the size of a reference is larger than the size of an
    /// `ErasedPtr`.
    /// To my knowledge, there are no pointer types with this property.
    pub fn new<T: ?Sized>(reference: NonNull<T>) -> Erased {
        let mut ptr = Erased([0; 2]);
        let ptr_size = size_of::<NonNull<T>>();
        // Extract out the pointer as raw memory
        assert!(
            ptr_size <= size_of::<Erased>(),
            "pointers to T are too big for storage"
        );
        unsafe {
            // SAFETY: We know that `cleanup` has at least as much space as `ptr_size`, and that
            // `box_ref` has size equal to `ptr_size`.
            copy_nonoverlapping(
                addr_of!(reference).cast::<u8>(),
                addr_of_mut!(ptr.0).cast::<u8>(),
                ptr_size,
            );
        }

        ptr
    }

    /// Specify this pointer into a pointer of a particular type.
    ///
    /// # Safety
    ///
    /// This function must only be specified to the type that the pointer was constructed with
    /// via [`ErasedPtr::new`].
    pub unsafe fn specify<T: ?Sized>(self) -> NonNull<T> {
        let mut box_ref: MaybeUninit<NonNull<T>> = MaybeUninit::zeroed();

        // For some reason, switching the ordering of casts causes this to create wacky undefined
        // behavior. Why? I don't know. I have better things to do than pontificate on this on a
        // Sunday afternoon.
        copy_nonoverlapping(
            addr_of!(self.0).cast::<u8>(),
            addr_of_mut!(box_ref).cast::<u8>(),
            size_of::<NonNull<T>>(),
        );

        box_ref.assume_init()
    }
}

impl fmt::Debug for Erased {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ErasedPtr({:x?})", self.0)
    }
}
