use std::ptr;

pub use std::collections::TryReserveError;
pub type TryReserveResult<T> = Result<T, TryReserveError>;

pub trait VecExt: Sized {
    type T;
    fn try_with_capacity(n: usize) -> TryReserveResult<Self>;
    fn try_push(&mut self, x: Self::T) -> TryReserveResult<()>;
    fn try_extend_from_slice_copy(&mut self, other: &[Self::T]) -> TryReserveResult<()>
    where
        Self::T: Copy;
    fn try_extend_from_slice_clone(&mut self, other: &[Self::T]) -> TryReserveResult<()>
    where
        Self::T: Clone;
    fn try_append(&mut self, other: &mut Self) -> TryReserveResult<()>;
}

impl<T> VecExt for Vec<T> {
    type T = T;
    fn try_with_capacity(n: usize) -> TryReserveResult<Self> {
        let mut v = Vec::new();
        v.try_reserve_exact(n)?;
        Ok(v)
    }
    fn try_push(&mut self, value: Self::T) -> TryReserveResult<()> {
        let len = self.len();
        if len == self.capacity() {
            self.try_reserve(1)?;
        }
        unsafe {
            let end = self.as_mut_ptr().add(len);
            ptr::write(end, value);
            self.set_len(len + 1);
        }
        Ok(())
    }
    fn try_extend_from_slice_copy(&mut self, other: &[T]) -> TryReserveResult<()>
    where
        T: Copy,
    {
        self.try_reserve(other.len())?;
        unsafe { unchecked_append(self, other) };
        Ok(())
    }
    fn try_extend_from_slice_clone(&mut self, other: &[Self::T]) -> TryReserveResult<()>
    where
        Self::T: Clone,
    {
        self.try_reserve(other.len())?;
        let len = self.len();
        unsafe {
            let mut local_len = SetLenOnDrop::new(self);
            let mut ptr = local_len.vec.as_mut_ptr().add(len);
            for element in other {
                ptr::write(ptr, element.clone());
                ptr = ptr.offset(1);
                local_len.increment_len(1);
            }
        }
        Ok(())
    }
    fn try_append(&mut self, other: &mut Self) -> TryReserveResult<()> {
        let other_len = other.len();
        self.try_reserve(other_len)?;
        unsafe {
            other.set_len(0);
            unchecked_append(self, std::slice::from_raw_parts(other.as_ptr(), other_len));
        }
        Ok(())
    }
}

struct SetLenOnDrop<'a, T> {
    vec: &'a mut Vec<T>,
    local_len: usize,
}

impl<'a, T> SetLenOnDrop<'a, T> {
    #[inline]
    fn new(vec: &'a mut Vec<T>) -> Self {
        SetLenOnDrop {
            local_len: vec.len(),
            vec,
        }
    }

    #[inline]
    fn increment_len(&mut self, increment: usize) {
        self.local_len += increment;
    }
}

impl<T> Drop for SetLenOnDrop<'_, T> {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.vec.set_len(self.local_len) };
    }
}

unsafe fn unchecked_append<T>(v: &mut Vec<T>, other: &[T]) {
    // modified from Vec::append_elements
    let count = other.len();
    let len = v.len();
    ptr::copy_nonoverlapping(other.as_ptr(), v.as_mut_ptr().add(len), count);
    v.set_len(len + count);
}

pub trait SliceExt {
    type Container;
    fn try_repeat(&self, n: usize) -> TryReserveResult<Self::Container>;
}

impl<T: Copy> SliceExt for [T] {
    type Container = Vec<T>;
    fn try_repeat(&self, n: usize) -> TryReserveResult<Self::Container> {
        // implementation modified from <[T]>::repeat

        if n == 0 {
            return Ok(Vec::new());
        }

        // If `n` is larger than zero, it can be split as
        // `n = 2^expn + rem (2^expn > rem, expn >= 0, rem >= 0)`.
        // `2^expn` is the number represented by the leftmost '1' bit of `n`,
        // and `rem` is the remaining part of `n`.

        // Using `Vec` to access `set_len()`.
        let capacity = self.len().checked_mul(n).ok_or_else(capacity_overflow)?;
        let mut buf = Vec::try_with_capacity(capacity)?;

        // `2^expn` repetition is done by doubling `buf` `expn`-times.
        unsafe { unchecked_append(&mut buf, self) };
        {
            let mut m = n >> 1;
            // If `m > 0`, there are remaining bits up to the leftmost '1'.
            while m > 0 {
                // `buf.extend(buf)`:
                unsafe {
                    ptr::copy_nonoverlapping(
                        buf.as_ptr(),
                        (buf.as_mut_ptr() as *mut T).add(buf.len()),
                        buf.len(),
                    );
                    // `buf` has capacity of `self.len() * n`.
                    let buf_len = buf.len();
                    buf.set_len(buf_len * 2);
                }

                m >>= 1;
            }
        }

        // `rem` (`= n - 2^expn`) repetition is done by copying
        // first `rem` repetitions from `buf` itself.
        let rem_len = capacity - buf.len(); // `self.len() * rem`
        if rem_len > 0 {
            // `buf.extend(buf[0 .. rem_len])`:
            unsafe {
                // This is non-overlapping since `2^expn > rem`.
                ptr::copy_nonoverlapping(
                    buf.as_ptr(),
                    (buf.as_mut_ptr() as *mut T).add(buf.len()),
                    rem_len,
                );
                // `buf.len() + rem_len` equals to `buf.capacity()` (`= self.len() * n`).
                buf.set_len(capacity);
            }
        }
        Ok(buf)
    }
}

impl SliceExt for str {
    type Container = String;
    fn try_repeat(&self, n: usize) -> TryReserveResult<Self::Container> {
        self.as_bytes()
            .try_repeat(n)
            .map(|v| unsafe { String::from_utf8_unchecked(v) })
    }
}

// a function to produce a capacity overflow error, even though creating a TryReserveError
// isn't stable. it optimizes to a no-op:
// rustpython_common::safe_alloc::capacity_overflow:
//  xor     edx, edx
//  ret
#[inline]
fn capacity_overflow() -> TryReserveError {
    let mut v = Vec::<()>::new();
    // equivalent to v.set_len(usize::MAX)
    v.extend(std::iter::repeat(()).take(usize::MAX));
    v.try_reserve_exact(usize::MAX).unwrap_err()
}
