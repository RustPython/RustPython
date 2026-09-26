// spell-checker:ignore pszSound fdwSound
#![allow(non_snake_case)]

pub(crate) use winsound::module_def;

#[pymodule]
mod winsound {
    use crate::builtins::{PyBaseExceptionRef, PyBytes, PyStr};
    use crate::convert::{IntoPyException, ToPyException};
    use crate::protocol::{BufferFlags, PyBuffer};
    use crate::{AsObject, PyObjectRef, PyResult, VirtualMachine};
    use rustpython_host_env::winsound as host_winsound;
    use rustpython_host_env::winsound::{PlaySoundError, PlaySoundSource, play_sound};

    #[pyattr]
    use host_winsound::{
        MB_ICONASTERISK, MB_ICONERROR, MB_ICONEXCLAMATION, MB_ICONHAND, MB_ICONINFORMATION,
        MB_ICONQUESTION, MB_ICONSTOP, MB_ICONWARNING, MB_OK, SND_ALIAS, SND_APPLICATION, SND_ASYNC,
        SND_FILENAME, SND_LOOP, SND_MEMORY, SND_NODEFAULT, SND_NOSTOP, SND_NOWAIT, SND_PURGE,
        SND_SENTRY, SND_SYNC, SND_SYSTEM,
    };

    #[derive(FromArgs)]
    struct PlaySoundArgs {
        #[pyarg(any)]
        sound: PyObjectRef,
        #[pyarg(any)]
        flags: i32,
    }

    fn map_play_err(vm: &VirtualMachine) -> impl FnOnce(PlaySoundError) -> PyBaseExceptionRef + '_ {
        |err| match err {
            PlaySoundError::MemoryAsyncRejected => {
                vm.new_runtime_error("Cannot play asynchronously from memory")
            }
            PlaySoundError::MemoryFlagWithoutBuffer | PlaySoundError::CallFailed => {
                vm.new_runtime_error("Failed to play sound")
            }
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
            let buffer = PyBuffer::from_object(vm, &sound, BufferFlags::SIMPLE)?;
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
            Some(s) => {
                let s = s.as_wtf8();
                let mut buf = Vec::with_capacity(s.len() + 1);
                buf.extend(s.encode_wide());
                buf
            }
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

                let s = result
                    .downcast_ref::<PyStr>()
                    .ok_or_else(|| {
                        vm.new_type_error(format!(
                            "expected {}.__fspath__() to return str or bytes, not {}",
                            sound.class().name(),
                            result.class().name()
                        ))
                    })?
                    .as_wtf8();

                let mut buf = Vec::with_capacity(s.len() + 1);
                buf.extend(s.encode_wide());
                buf
            }
        };

        // Check for embedded null characters
        let wide_cstr =
            widestring::WideCString::from_vec(path).map_err(|e| e.to_pyexception(vm))?;
        play_sound(PlaySoundSource::Name(&wide_cstr), flags).map_err(map_play_err(vm))
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
        #[pyarg(any, default)]
        r#type: u32,
    }

    #[pyfunction]
    fn MessageBeep(args: MessageBeepArgs, vm: &VirtualMachine) -> PyResult<()> {
        rustpython_host_env::winsound::message_beep(args.r#type).map_err(|e| e.into_pyexception(vm))
    }
}
