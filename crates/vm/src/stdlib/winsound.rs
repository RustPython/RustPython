// spell-checker:ignore pszSound fdwSound
#![allow(non_snake_case)]

pub(crate) use winsound::module_def;

#[pymodule]
mod winsound {
    use crate::builtins::{PyBytes, PyStr};
    use crate::convert::{IntoPyException, TryFromBorrowedObject};
    use crate::host_env::windows::ToWideString;
    use crate::protocol::PyBuffer;
    use crate::{AsObject, PyObjectRef, PyResult, VirtualMachine};
    use rustpython_host_env::winsound::{PlaySoundSource, play_sound};

    // PlaySound flags
    #[pyattr]
    const SND_SYNC: u32 = 0x0000;
    #[pyattr]
    const SND_ASYNC: u32 = 0x0001;
    #[pyattr]
    const SND_NODEFAULT: u32 = 0x0002;
    #[pyattr]
    const SND_MEMORY: u32 = 0x0004;
    #[pyattr]
    const SND_LOOP: u32 = 0x0008;
    #[pyattr]
    const SND_NOSTOP: u32 = 0x0010;
    #[pyattr]
    const SND_PURGE: u32 = 0x0040;
    #[pyattr]
    const SND_APPLICATION: u32 = 0x0080;
    #[pyattr]
    const SND_NOWAIT: u32 = 0x00002000;
    #[pyattr]
    const SND_ALIAS: u32 = 0x00010000;
    #[pyattr]
    const SND_FILENAME: u32 = 0x00020000;
    #[pyattr]
    const SND_SENTRY: u32 = 0x00080000;
    #[pyattr]
    const SND_SYSTEM: u32 = 0x00200000;

    // MessageBeep types
    #[pyattr]
    const MB_OK: u32 = 0x00000000;
    #[pyattr]
    const MB_ICONHAND: u32 = 0x00000010;
    #[pyattr]
    const MB_ICONQUESTION: u32 = 0x00000020;
    #[pyattr]
    const MB_ICONEXCLAMATION: u32 = 0x00000030;
    #[pyattr]
    const MB_ICONASTERISK: u32 = 0x00000040;
    #[pyattr]
    const MB_ICONERROR: u32 = MB_ICONHAND;
    #[pyattr]
    const MB_ICONSTOP: u32 = MB_ICONHAND;
    #[pyattr]
    const MB_ICONINFORMATION: u32 = MB_ICONASTERISK;
    #[pyattr]
    const MB_ICONWARNING: u32 = MB_ICONEXCLAMATION;

    #[derive(FromArgs)]
    struct PlaySoundArgs {
        #[pyarg(any)]
        sound: PyObjectRef,
        #[pyarg(any)]
        flags: i32,
    }

    fn map_play_err(
        vm: &VirtualMachine,
    ) -> impl FnOnce(
        rustpython_host_env::winsound::PlaySoundError,
    ) -> crate::builtins::PyBaseExceptionRef
    + '_ {
        use rustpython_host_env::winsound::PlaySoundError::*;
        |err| match err {
            MemoryAsyncRejected => vm.new_runtime_error("Cannot play asynchronously from memory"),
            MemoryFlagWithoutBuffer | CallFailed => vm.new_runtime_error("Failed to play sound"),
        }
    }

    #[pyfunction]
    fn PlaySound(args: PlaySoundArgs, vm: &VirtualMachine) -> PyResult<()> {
        let sound = args.sound;
        let flags = args.flags as u32;

        if vm.is_none(&sound) {
            return play_sound(PlaySoundSource::Stop, flags).map_err(map_play_err(vm));
        }

        if flags & SND_MEMORY != 0 {
            let buffer = PyBuffer::try_from_borrowed_object(vm, &sound)?;
            let buf = buffer
                .as_contiguous()
                .ok_or_else(|| vm.new_type_error("a bytes-like object is required, not 'str'"))?;
            return play_sound(PlaySoundSource::Memory(&buf), flags).map_err(map_play_err(vm));
        }

        if sound.downcastable::<PyBytes>() {
            let type_name = sound.class().name().to_string();
            return Err(vm.new_type_error(format!(
                "'sound' must be str, os.PathLike, or None, not {type_name}"
            )));
        }

        // os.fspath(sound)
        let path = match sound.downcast_ref::<PyStr>() {
            Some(s) => s.as_wtf8().to_owned(),
            None => {
                let fspath = vm.get_method_or_type_error(
                    sound.clone(),
                    identifier!(vm, __fspath__),
                    || {
                        let type_name = sound.class().name().to_string();
                        format!("'sound' must be str, os.PathLike, or None, not {type_name}")
                    },
                )?;

                if vm.is_none(&fspath) {
                    return Err(vm.new_type_error(format!(
                        "'sound' must be str, os.PathLike, or None, not {}",
                        sound.class().name()
                    )));
                }
                let result = fspath.call((), vm)?;

                if result.downcastable::<PyBytes>() {
                    return Err(vm.new_type_error("'sound' must resolve to str, not bytes"));
                }

                let s: &PyStr = result.downcast_ref().ok_or_else(|| {
                    vm.new_type_error(format!(
                        "expected {}.__fspath__() to return str or bytes, not {}",
                        sound.class().name(),
                        result.class().name()
                    ))
                })?;

                s.as_wtf8().to_owned()
            }
        };

        // Check for embedded null characters
        if path.as_bytes().contains(&0) {
            return Err(vm.new_value_error("embedded null character"));
        }

        let wide = path.to_wide_with_nul();
        let wide_cstr = widestring::WideCStr::from_slice_truncate(&wide)
            .map_err(|_| vm.new_value_error("embedded null character"))?;
        play_sound(PlaySoundSource::Name(wide_cstr), flags).map_err(map_play_err(vm))
    }

    #[derive(FromArgs)]
    struct BeepArgs {
        #[pyarg(any)]
        frequency: i32,
        #[pyarg(any)]
        duration: i32,
    }

    #[pyfunction]
    fn Beep(args: BeepArgs, vm: &VirtualMachine) -> PyResult<()> {
        if !(37..=32767).contains(&args.frequency) {
            return Err(vm.new_value_error("frequency must be in 37 thru 32767"));
        }

        if rustpython_host_env::winsound::beep(args.frequency as u32, args.duration as u32) {
            Ok(())
        } else {
            Err(vm.new_runtime_error("Failed to beep"))
        }
    }

    #[derive(FromArgs)]
    struct MessageBeepArgs {
        #[pyarg(any, default = 0)]
        r#type: u32,
    }

    #[pyfunction]
    fn MessageBeep(args: MessageBeepArgs, vm: &VirtualMachine) -> PyResult<()> {
        rustpython_host_env::winsound::message_beep(args.r#type).map_err(|e| e.into_pyexception(vm))
    }
}
