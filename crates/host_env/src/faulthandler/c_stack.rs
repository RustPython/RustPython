use core::fmt::{self, Write};

struct Output(i32);

impl Output {
    fn bytes(&mut self, bytes: &[u8]) {
        // Like CPython's diagnostic writes, output errors are best effort.
        let _ = crate::crt_fd::write_noraise(self.0, bytes);
    }
}

impl Write for Output {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.bytes(text.as_bytes());
        Ok(())
    }
}

/// Dump the calling thread's native stack to a borrowed file descriptor.
///
/// This is only for normal execution, never a signal or exception handler:
/// unwinding and loader lookup may allocate or acquire internal locks. The
/// descriptor remains owned by the caller and must stay open during this call.
pub fn dump_c_stack(fd: i32) {
    let mut output = Output(fd);
    output.bytes(b"Current thread's C stack trace (most recent call first):\n");
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    native::dump(&mut output);
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    output.bytes(b"  <cannot get C stack on this system>\n");
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod native {
    use super::Output;
    use alloc::vec::Vec;
    use core::{ffi::CStr, fmt::Write, ptr};
    use std::{os::unix::ffi::OsStrExt, path::PathBuf};

    const BACKTRACE_SIZE: usize = 32;

    #[derive(Default)]
    struct Frame {
        bias: Option<usize>,
        filename: Vec<u8>,
        copy_failed: bool,
    }

    struct Frames {
        addresses: [*mut libc::c_void; BACKTRACE_SIZE],
        details: [Frame; BACKTRACE_SIZE],
        executable: Option<PathBuf>,
        count: usize,
    }

    // ELF addresses have the target's pointer width. Use a wider intermediate
    // and checked conversion so malformed or overflowing ranges are ignored.
    fn load_range(
        bias: impl Into<u64>,
        address: impl Into<u64>,
        size: impl Into<u64>,
    ) -> Option<(usize, usize)> {
        let start = bias.into().checked_add(address.into())?;
        let end = start.checked_add(size.into())?;
        Some((usize::try_from(start).ok()?, usize::try_from(end).ok()?))
    }

    unsafe extern "C" fn collect_frames(
        info: *mut libc::dl_phdr_info,
        _size: libc::size_t,
        data: *mut libc::c_void,
    ) -> libc::c_int {
        // dl_iterate_phdr invokes this synchronously while the GNU loader keeps
        // these records, headers, and names alive. data is the caller's exclusive
        // live Frames. Only owned filename bytes escape this callback.
        let info = unsafe { &*info };
        let frames = unsafe { &mut *data.cast::<Frames>() };
        if info.dlpi_phdr.is_null() {
            return 0;
        }
        let filename = if info.dlpi_name.is_null() {
            &[][..]
        } else {
            unsafe { CStr::from_ptr(info.dlpi_name) }.to_bytes()
        };
        // The main executable has an empty loader name. Resolve its actual path
        // before taking the loader lock, rather than doing filesystem I/O here.
        let filename = if filename.is_empty() {
            frames
                .executable
                .as_ref()
                .map_or(&[][..], |path| path.as_os_str().as_bytes())
        } else {
            filename
        };
        // libc selects Elf32_Addr or Elf64_Addr to match the pointer width.
        let bias = info.dlpi_addr as usize;
        // The loader supplies exactly dlpi_phnum initialized program headers.
        let headers =
            unsafe { core::slice::from_raw_parts(info.dlpi_phdr, usize::from(info.dlpi_phnum)) };
        for header in headers {
            if header.p_type != libc::PT_LOAD {
                continue;
            }
            let Some((start, end)) = load_range(info.dlpi_addr, header.p_vaddr, header.p_memsz)
            else {
                continue;
            };
            for (address, frame) in frames
                .addresses
                .iter()
                .zip(frames.details.iter_mut())
                .take(frames.count)
            {
                let address = *address as usize;
                if frame.bias.is_some() || address < start || address >= end {
                    continue;
                }
                frame.bias = Some(bias);
                // Allocation failure is recorded, not unwound through the C
                // callback. After reservation, copying bytes cannot allocate.
                if frame.filename.try_reserve_exact(filename.len()).is_err() {
                    frame.copy_failed = true;
                } else {
                    frame.filename.extend_from_slice(filename);
                }
            }
        }
        0
    }

    fn write_frame(output: &mut Output, address: usize, frame: &Frame) {
        if frame.copy_failed {
            let _ = writeln!(output, "  <unable to copy binary filename> [{address:#x}]");
            return;
        }
        if frame.filename.is_empty() {
            let _ = writeln!(output, "  Binary file '<unknown>' [{address:#x}]");
            return;
        }
        output.bytes(b"  Binary file \"");
        output.bytes(&frame.filename);
        output.bytes(b"\"");
        // A zero ELF load bias (e.g. non-PIE executable) uses CPython's
        // filename/address-only form. Do not substitute a mapping address.
        if let Some(base) = frame.bias.filter(|&bias| bias != 0) {
            let (sign, offset) = if address >= base {
                ('+', address - base)
            } else {
                ('-', base - address)
            };
            let _ = write!(output, ", at {sign}{offset:#x}");
        }
        let _ = writeln!(output, " [{address:#x}]");
    }

    pub(super) fn dump(output: &mut Output) {
        let mut frames = Frames {
            addresses: [ptr::null_mut(); BACKTRACE_SIZE],
            details: core::array::from_fn(|_| Frame::default()),
            executable: std::env::current_exe().ok(),
            count: 0,
        };
        // The buffer has room for the exact number of pointers passed to libc.
        let count = unsafe {
            libc::backtrace(frames.addresses.as_mut_ptr(), BACKTRACE_SIZE as libc::c_int)
        };
        frames.count = count.clamp(0, BACKTRACE_SIZE as libc::c_int) as usize;
        if frames.count == 0 {
            output.bytes(b"  <system returned no stack trace>\n");
            return;
        }
        // Callback state lives until this synchronous traversal returns. The
        // callback performs only checked collection and fallible byte copies,
        // never output or Python calls while the loader lock is held.
        // Do not call dladdr here: its returned strings can be invalidated by
        // dlclose after it returns, and nesting it inside dl_iterate_phdr would
        // invert glibc's dl_load_lock/dl_load_write_lock order. Report genuine
        // object-relative offsets without optional symbol names instead.
        unsafe {
            libc::dl_iterate_phdr(Some(collect_frames), ptr::from_mut(&mut frames).cast());
        }
        for (address, frame) in frames
            .addresses
            .iter()
            .zip(frames.details.iter())
            .take(frames.count)
        {
            write_frame(output, *address as usize, frame);
        }
        if frames.count == BACKTRACE_SIZE {
            output.bytes(b"  <truncated rest of calls>\n");
        }
    }
}
