// spell-checker:ignore pszSound fdwSound winmm

use std::io;

#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(pszSound: *const u16, hmod: isize, fdwSound: u32) -> i32;
}

unsafe extern "system" {
    fn Beep(dwFreq: u32, dwDuration: u32) -> i32;
    fn MessageBeep(uType: u32) -> i32;
}

/// `SND_ASYNC` flag value from `mmsystem.h`.
const SND_ASYNC: u32 = 0x0001;
/// `SND_MEMORY` flag value from `mmsystem.h`.
const SND_MEMORY: u32 = 0x0004;

/// Source for a `PlaySound` call.
pub enum PlaySoundSource<'a> {
    /// Stop currently playing sound (NULL `pszSound`).
    Stop,
    /// Play sound data from memory; pass with `SND_MEMORY` set in `flags`.
    Memory(&'a [u8]),
    /// Play sound by filename or system alias.
    Name(&'a widestring::WideCStr),
}

/// Returns `Ok(())` when `PlaySoundW` returns non-zero, an error otherwise.
///
/// Rejects `Memory(_)` together with `SND_ASYNC` because async playback
/// requires the buffer to outlive the call; combining them with a borrowed
/// slice would let WinMM read freed memory.
pub fn play_sound(source: PlaySoundSource<'_>, flags: u32) -> Result<(), PlaySoundError> {
    if matches!(source, PlaySoundSource::Memory(_)) && flags & SND_ASYNC != 0 {
        return Err(PlaySoundError::MemoryAsyncRejected);
    }
    // `SND_MEMORY` requires a `Memory(_)` source; an empty pointer would
    // dereference garbage.
    if !matches!(source, PlaySoundSource::Memory(_)) && flags & SND_MEMORY != 0 {
        return Err(PlaySoundError::MemoryFlagWithoutBuffer);
    }
    let ptr: *const u16 = match source {
        PlaySoundSource::Stop => core::ptr::null(),
        PlaySoundSource::Memory(buf) => buf.as_ptr().cast(),
        PlaySoundSource::Name(s) => s.as_ptr(),
    };
    let ok = unsafe { PlaySoundW(ptr, 0, flags) };
    if ok == 0 {
        Err(PlaySoundError::CallFailed)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PlaySoundError {
    /// `PlaySoundW` returned 0; there is no documented errno for this path.
    CallFailed,
    /// `Memory(_)` source cannot be combined with `SND_ASYNC` in the safe API.
    MemoryAsyncRejected,
    /// `SND_MEMORY` set in `flags` but no `Memory(_)` buffer supplied.
    MemoryFlagWithoutBuffer,
}

/// `Beep(freq, duration)`. `false` on failure.
#[must_use]
pub fn beep(frequency: u32, duration_ms: u32) -> bool {
    unsafe { Beep(frequency, duration_ms) != 0 }
}

/// `MessageBeep(type)`. On failure returns `Err` populated from `GetLastError`.
pub fn message_beep(beep_type: u32) -> io::Result<()> {
    let ok = unsafe { MessageBeep(beep_type) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
