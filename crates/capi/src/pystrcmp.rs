use core::ffi::c_char;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_mystricmp(str1: *const c_char, str2: *const c_char) -> i32 {
    unsafe { PyOS_mystrnicmp(str1, str2, isize::MAX) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_mystrnicmp(
    str1: *const c_char,
    str2: *const c_char,
    size: isize,
) -> i32 {
    let Ok(limit) = usize::try_from(size) else {
        return 0;
    };
    let mut index = 0usize;
    while index < limit {
        let left = unsafe { *str1.add(index) } as u8;
        let right = unsafe { *str2.add(index) } as u8;
        let diff = left.to_ascii_lowercase() as i32 - right.to_ascii_lowercase() as i32;
        if diff != 0 || left == 0 || right == 0 {
            return diff;
        }
        index += 1;
    }
    0
}
