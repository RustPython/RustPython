// cspell:ignore decerror encerror pendingsize statebytes statelong mbc
pub(crate) use _multibytecodec::{get_codec, module_def};

#[pymodule]
mod _multibytecodec {
    use crate::common::{
        encodings::cjk::{self, Codec, DecodeOne, EncodeOne},
        lock::PyMutex,
        wtf8::{CodePoint, Wtf8Buf},
    };
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyBytes, PyInt, PyStr, PyStrRef, PyTuple, PyType},
        class::PyClassImpl,
        function::{ArgBytesLike, FuncArgs, OptionalArg, OptionalOption, PySetterValue},
        protocol::PySequence,
        types::{Constructor, Initializer},
    };
    use malachite_bigint::{BigInt, Sign};

    /// Pending code points an encoder may hold between calls.
    const MAXENCPENDING: usize = 2;
    /// Pending bytes a decoder may hold between calls.
    const MAXDECPENDING: usize = 8;
    /// A pending size byte, the UTF-8 of the pending characters, then the state.
    const ENCODER_STATE_SIZE: usize = 1 + MAXENCPENDING * 4 + 8;

    fn code_points(s: &Py<PyStr>) -> Vec<u32> {
        s.as_wtf8().code_points().map(CodePoint::to_u32).collect()
    }

    fn push_code_point(out: &mut Wtf8Buf, c: u32) {
        out.push(CodePoint::from_u32(c).expect("codec produced an invalid code point"));
    }

    #[derive(Debug, Clone)]
    enum ErrorHandler {
        Strict,
        Ignore,
        Replace,
        Custom(PyStrRef),
    }

    impl ErrorHandler {
        fn new(errors: Option<PyStrRef>) -> Self {
            let Some(errors) = errors else {
                return Self::Strict;
            };
            match errors.to_str() {
                Some("strict") => Self::Strict,
                Some("ignore") => Self::Ignore,
                Some("replace") => Self::Replace,
                _ => Self::Custom(errors),
            }
        }

        fn name(&self, vm: &VirtualMachine) -> PyStrRef {
            match self {
                Self::Strict => vm.ctx.new_str("strict"),
                Self::Ignore => vm.ctx.new_str("ignore"),
                Self::Replace => vm.ctx.new_str("replace"),
                Self::Custom(name) => name.clone(),
            }
        }

        fn call(&self, exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyResult {
            let Self::Custom(name) = self else {
                unreachable!("built-in handlers never reach the callback")
            };
            let handler = vm
                .state
                .codec_registry
                .lookup_error(&name.to_string_lossy(), vm)?;
            handler.call((exc,), vm)
        }
    }

    /// The position an error handler asked the driver to resume from.
    fn resume_position(newpos: &PyObject, inlen: usize, vm: &VirtualMachine) -> PyResult<usize> {
        let inlen = inlen as isize;
        let (newpos, converted) = match newpos.try_index(vm)?.try_to_primitive::<isize>(vm) {
            Ok(pos) => (pos, true),
            Err(_) => (-1, false),
        };
        let newpos = if newpos < 0 && converted {
            newpos + inlen
        } else {
            newpos
        };
        if newpos < 0 || newpos > inlen {
            return Err(vm.new_index_error(format!(
                "position {newpos} from error handler out of bounds"
            )));
        }
        Ok(newpos as usize)
    }

    fn unpack_handler_result<'a>(
        result: &'a PyObject,
        message: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<(&'a PyObject, &'a PyObject)> {
        let error = || vm.new_type_error(message);
        let tuple = result.downcast_ref::<PyTuple>().ok_or_else(error)?;
        let [replacement, newpos] = tuple.as_slice() else {
            return Err(error());
        };
        if !newpos.downcastable::<PyInt>() {
            return Err(error());
        }
        Ok((replacement, newpos))
    }

    /// `multibytecodec.c::MultibyteEncodeBuffer`.
    struct EncodeBuffer {
        input: PyStrRef,
        chars: Vec<u32>,
        pos: usize,
        out: Vec<u8>,
        exception: Option<PyBaseExceptionRef>,
    }

    impl EncodeBuffer {
        fn new(input: PyStrRef) -> Self {
            let chars = code_points(&input);
            Self {
                input,
                chars,
                pos: 0,
                out: Vec::new(),
                exception: None,
            }
        }

        fn error(
            &mut self,
            encoding: &str,
            start: usize,
            end: usize,
            reason: &str,
            vm: &VirtualMachine,
        ) -> PyResult<PyBaseExceptionRef> {
            if let Some(exc) = &self.exception {
                let obj = exc.as_object();
                obj.set_attr("start", vm.ctx.new_int(start), vm)?;
                obj.set_attr("end", vm.ctx.new_int(end), vm)?;
                obj.set_attr("reason", vm.ctx.new_str(reason), vm)?;
                return Ok(exc.clone());
            }
            let exc = vm.new_unicode_encode_error_real(
                vm.ctx.new_str(encoding),
                self.input.clone(),
                start,
                end,
                vm.ctx.new_str(reason),
            );
            Ok(self.exception.insert(exc).clone())
        }
    }

    /// `multibytecodec.c::MultibyteDecodeBuffer`.
    struct DecodeBuffer {
        data: Vec<u8>,
        pos: usize,
        out: Wtf8Buf,
        exception: Option<PyBaseExceptionRef>,
    }

    impl DecodeBuffer {
        fn error(
            &mut self,
            encoding: &str,
            start: usize,
            end: usize,
            reason: &str,
            vm: &VirtualMachine,
        ) -> PyResult<PyBaseExceptionRef> {
            if let Some(exc) = &self.exception {
                let obj = exc.as_object();
                obj.set_attr("start", vm.ctx.new_int(start), vm)?;
                obj.set_attr("end", vm.ctx.new_int(end), vm)?;
                obj.set_attr("reason", vm.ctx.new_str(reason), vm)?;
                return Ok(exc.clone());
            }
            let exc = vm.new_unicode_decode_error(
                vm.ctx.new_str(encoding),
                vm.ctx.new_bytes(self.data.clone()),
                start,
                end,
                vm.ctx.new_str(reason),
            );
            Ok(self.exception.insert(exc).clone())
        }
    }

    /// `multibytecodec.c::multibytecodec_encerror`.
    fn encode_error(
        codec: CodecRef,
        state: &mut [u8; 8],
        buf: &mut EncodeBuffer,
        errors: &ErrorHandler,
        esize: usize,
        reason: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let ErrorHandler::Replace = errors {
            match cjk::encode_one(codec.codec, &['?' as u32], false, state) {
                EncodeOne::Bytes(bytes, len, _) => buf.out.extend_from_slice(&bytes[..len]),
                _ => buf.out.push(b'?'),
            }
        }
        if let ErrorHandler::Ignore | ErrorHandler::Replace = errors {
            buf.pos += esize;
            return Ok(());
        }

        let start = buf.pos;
        let exc = buf.error(codec.encoding, start, start + esize, reason, vm)?;
        if let ErrorHandler::Strict = errors {
            return Err(exc);
        }

        let result = errors.call(exc, vm)?;
        let (replacement, newpos) = unpack_handler_result(
            &result,
            "encoding error handler must return (str, int) tuple",
            vm,
        )?;
        let replacement = if let Some(s) = replacement.downcast_ref::<PyStr>() {
            // The handler's text has to survive the codec itself.
            encode(
                codec,
                state,
                s.to_owned(),
                &ErrorHandler::Strict,
                true,
                false,
                vm,
            )?
            .0
        } else if let Some(bytes) = replacement.downcast_ref::<PyBytes>() {
            bytes.as_bytes().to_vec()
        } else {
            return Err(vm.new_type_error("encoding error handler must return (str, int) tuple"));
        };
        buf.out.extend_from_slice(&replacement);
        buf.pos = resume_position(newpos, buf.chars.len(), vm)?;
        Ok(())
    }

    /// `multibytecodec.c::multibytecodec_decerror`.
    fn decode_error(
        encoding: &str,
        buf: &mut DecodeBuffer,
        errors: &ErrorHandler,
        esize: usize,
        reason: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let ErrorHandler::Replace = errors {
            buf.out.push_char(char::REPLACEMENT_CHARACTER);
        }
        if let ErrorHandler::Ignore | ErrorHandler::Replace = errors {
            buf.pos += esize;
            return Ok(());
        }

        let start = buf.pos;
        let exc = buf.error(encoding, start, start + esize, reason, vm)?;
        if let ErrorHandler::Strict = errors {
            return Err(exc);
        }

        let result = errors.call(exc, vm)?;
        let (replacement, newpos) = unpack_handler_result(
            &result,
            "decoding error handler must return (str, int) tuple",
            vm,
        )?;
        let replacement = replacement.downcast_ref::<PyStr>().ok_or_else(|| {
            vm.new_type_error("decoding error handler must return (str, int) tuple")
        })?;
        buf.out.push_wtf8(replacement.as_wtf8());
        buf.pos = resume_position(newpos, buf.data.len(), vm)?;
        Ok(())
    }

    /// `multibytecodec.c::multibytecodec_encode`.
    #[allow(clippy::too_many_arguments)]
    fn encode(
        codec: CodecRef,
        state: &mut [u8; 8],
        input: PyStrRef,
        errors: &ErrorHandler,
        flush: bool,
        reset: bool,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, usize)> {
        let mut buf = EncodeBuffer::new(input);
        if buf.chars.is_empty() && !reset {
            return Ok((Vec::new(), 0));
        }

        while buf.pos < buf.chars.len() {
            match cjk::encode_one(codec.codec, &buf.chars[buf.pos..], flush, state) {
                EncodeOne::Bytes(bytes, len, consumed) => {
                    buf.out.extend_from_slice(&bytes[..len]);
                    buf.pos += consumed;
                }
                EncodeOne::Incomplete => {
                    if !flush {
                        break;
                    }
                    // The size the engine reports for a truncated tail.
                    let esize = buf.pos;
                    encode_error(
                        codec,
                        state,
                        &mut buf,
                        errors,
                        esize,
                        "incomplete multibyte sequence",
                        vm,
                    )?;
                    break;
                }
                EncodeOne::Illegal(esize) => encode_error(
                    codec,
                    state,
                    &mut buf,
                    errors,
                    esize,
                    "illegal multibyte sequence",
                    vm,
                )?,
            }
        }

        if reset && let Some((bytes, len)) = cjk::encode_reset(codec.codec, state) {
            buf.out.extend_from_slice(&bytes[..len]);
        }
        Ok((buf.out, buf.pos))
    }

    /// `multibytecodec.c::decoder_feed_buffer`, which stops on a truncated tail,
    /// and the driving loop of `MultibyteCodec.decode`, which reports one.
    fn decode_into(
        codec: CodecRef,
        state: &mut [u8; 8],
        buf: &mut DecodeBuffer,
        errors: &ErrorHandler,
        keep_incomplete: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        while buf.pos < buf.data.len() {
            match cjk::decode_one(codec.codec, &buf.data[buf.pos..], state) {
                DecodeOne::Char(c, consumed) => {
                    push_code_point(&mut buf.out, c);
                    buf.pos += consumed;
                }
                DecodeOne::Pair(first, second, consumed) => {
                    push_code_point(&mut buf.out, first);
                    push_code_point(&mut buf.out, second);
                    buf.pos += consumed;
                }
                DecodeOne::Skip(consumed) => buf.pos += consumed,
                DecodeOne::Incomplete => {
                    if keep_incomplete {
                        break;
                    }
                    decode_incomplete(codec.encoding, buf, errors, vm)?;
                }
                DecodeOne::Illegal(esize) => decode_error(
                    codec.encoding,
                    buf,
                    errors,
                    esize,
                    "illegal multibyte sequence",
                    vm,
                )?,
            }
        }
        Ok(())
    }

    fn decode_incomplete(
        encoding: &str,
        buf: &mut DecodeBuffer,
        errors: &ErrorHandler,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let esize = buf.data.len() - buf.pos;
        decode_error(
            encoding,
            buf,
            errors,
            esize,
            "incomplete multibyte sequence",
            vm,
        )
    }

    #[derive(Debug, Clone)]
    struct EncoderState {
        state: [u8; 8],
        pending: Option<PyStrRef>,
        errors: ErrorHandler,
    }

    #[derive(Debug, Clone)]
    struct DecoderState {
        state: [u8; 8],
        pending: Vec<u8>,
        errors: ErrorHandler,
    }

    /// The codec a stateful object was built from, taken off its class.
    #[derive(Debug, Clone, Copy)]
    struct CodecRef {
        codec: Codec,
        encoding: &'static str,
    }

    impl CodecRef {
        /// `mbiencoder_new`'s `PyObject_GetAttrString(type, "codec")` check.
        fn from_class(cls: &Py<PyType>, vm: &VirtualMachine) -> PyResult<Self> {
            let codec = cls.as_object().get_attr("codec", vm)?;
            let codec = codec
                .downcast_ref::<MultibyteCodec>()
                .ok_or_else(|| vm.new_type_error("codec is unexpected type"))?;
            Ok(codec.codec)
        }

        fn encoder_state(&self, errors: Option<PyStrRef>) -> EncoderState {
            EncoderState {
                state: cjk::initial_state(self.codec, false),
                pending: None,
                errors: ErrorHandler::new(errors),
            }
        }

        fn decoder_state(&self, errors: Option<PyStrRef>) -> DecoderState {
            DecoderState {
                state: cjk::initial_state(self.codec, true),
                pending: Vec::new(),
                errors: ErrorHandler::new(errors),
            }
        }

        /// `encoder_encode_stateful`.
        fn encode_stateful(
            &self,
            inner: &mut EncoderState,
            input: PyObjectRef,
            final_input: bool,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            let input = to_text(input, vm)?;
            let original_pending = inner.pending.take();
            let input = match &original_pending {
                Some(pending) => {
                    let mut joined = pending.as_wtf8().to_owned();
                    joined.push_wtf8(input.as_wtf8());
                    vm.ctx.new_str(joined)
                }
                None => input,
            };

            let chars = input.char_len();
            let encoded = encode(
                *self,
                &mut inner.state,
                input.clone(),
                &inner.errors,
                final_input,
                final_input,
                vm,
            );
            let (out, pos) = match encoded {
                Ok(encoded) => encoded,
                Err(e) => {
                    inner.pending = original_pending;
                    return Err(e);
                }
            };

            if pos < chars {
                if chars - pos > MAXENCPENDING {
                    // Normal codecs can't reach here.
                    return Err(vm.new_unicode_encode_error_real(
                        vm.ctx.new_str(self.encoding),
                        input,
                        pos,
                        chars,
                        vm.ctx.new_str("pending buffer overflow"),
                    ));
                }
                let tail: Wtf8Buf = input.as_wtf8().code_points().skip(pos).collect();
                inner.pending = Some(vm.ctx.new_str(tail));
            }
            Ok(out)
        }

        /// `decoder_append_pending`.
        fn append_pending(
            &self,
            inner: &mut DecoderState,
            buf: &DecodeBuffer,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let tail = &buf.data[buf.pos..];
            if tail.len() + inner.pending.len() > MAXDECPENDING {
                return Err(vm.new_unicode_decode_error(
                    vm.ctx.new_str(self.encoding),
                    vm.ctx.new_bytes(buf.data.clone()),
                    0,
                    buf.data.len(),
                    vm.ctx.new_str("pending buffer overflow"),
                ));
            }
            inner.pending.extend_from_slice(tail);
            Ok(())
        }
    }

    /// `PyObject_Str` for an object a codec was handed instead of a string.
    fn to_text(input: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        match input.downcast::<PyStr>() {
            Ok(s) => Ok(s),
            Err(obj) => obj.str(vm),
        }
    }

    fn state_to_int(state: &[u8]) -> BigInt {
        BigInt::from_bytes_le(Sign::Plus, state)
    }

    fn int_to_state(value: &Py<PyInt>, size: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let value = value.as_bigint();
        if value.sign() == Sign::Minus {
            return Err(vm.new_overflow_error("can't convert negative int to unsigned"));
        }
        let (_, mut bytes) = value.to_bytes_le();
        if bytes.len() > size {
            return Err(vm.new_overflow_error("int too big to convert"));
        }
        bytes.resize(size, 0);
        Ok(bytes)
    }

    #[pyclass(no_attr, module = "_multibytecodec", name = "MultibyteCodec")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct MultibyteCodec {
        codec: CodecRef,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION, IMMUTABLETYPE))]
    impl MultibyteCodec {
        #[pymethod]
        fn encode(&self, args: CodecEncodeArgs, vm: &VirtualMachine) -> PyResult {
            let CodecEncodeArgs { input, errors } = args;
            let input = to_text(input, vm)?;
            let chars = input.char_len();
            let errors = ErrorHandler::new(errors.flatten());
            let mut state = cjk::initial_state(self.codec.codec, false);
            let (out, _) = encode(self.codec, &mut state, input, &errors, true, true, vm)?;
            Ok(vm.new_tuple((vm.ctx.new_bytes(out), chars)).into())
        }

        #[pymethod]
        fn decode(&self, args: CodecDecodeArgs, vm: &VirtualMachine) -> PyResult {
            let CodecDecodeArgs { input, errors } = args;
            let data = input.borrow_buf().to_vec();
            let len = data.len();
            if len == 0 {
                return Ok(vm.new_tuple((vm.ctx.new_str(""), 0)).into());
            }
            let errors = ErrorHandler::new(errors.flatten());
            let mut state = cjk::initial_state(self.codec.codec, true);
            let mut buf = DecodeBuffer {
                data,
                pos: 0,
                out: Wtf8Buf::new(),
                exception: None,
            };
            decode_into(self.codec, &mut state, &mut buf, &errors, false, vm)?;
            Ok(vm.new_tuple((vm.ctx.new_str(buf.out), len)).into())
        }
    }

    /// `cjkcodecs.h::getcodec`, collapsed onto the codec object the capsule
    /// would have carried.
    pub(crate) fn get_codec(
        names: &[&'static str],
        encoding: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult {
        let name = encoding
            .downcast_ref::<PyStr>()
            .ok_or_else(|| vm.new_type_error("encoding name must be a string."))?;
        let name = name.to_str().unwrap_or_default();
        let codec = names
            .iter()
            .find(|candidate| **candidate == name)
            .and_then(|name| Codec::from_name(name).map(|codec| (codec, *name)));
        let Some((codec, encoding)) = codec else {
            return Err(vm.new_lookup_error("no such codec is supported."));
        };
        // Keep the type alive whichever module was imported first.
        let class = <MultibyteCodec as PyClassImpl>::make_static_type();
        let payload = MultibyteCodec {
            codec: CodecRef { codec, encoding },
        };
        Ok(PyRef::new_ref(payload, class, None).into())
    }

    /// `__create_codec`, which turns the capsule `getcodec` hands it into a
    /// codec object. Nothing here carries a codec in a capsule, so the argument
    /// check rejects everything a caller can pass.
    #[pyfunction(name = "__create_codec")]
    fn create_codec(_arg: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_value_error("argument type invalid"))
    }

    #[derive(FromArgs)]
    struct StatefulArgs {
        #[pyarg(any, optional)]
        errors: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct StreamArgs {
        #[pyarg(any)]
        stream: PyObjectRef,
        #[pyarg(any, optional)]
        errors: OptionalArg<PyObjectRef>,
    }

    /// The `|s` conversion the four stateful constructors share.
    fn errors_arg(
        errors: OptionalArg<PyObjectRef>,
        label: &str,
        index: usize,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyStrRef>> {
        let Some(errors) = errors.into_option() else {
            return Ok(None);
        };
        errors.downcast::<PyStr>().map(Some).map_err(|errors| {
            let kind = if vm.is_none(&errors) {
                "None".to_owned()
            } else {
                errors.class().name().to_string()
            };
            vm.new_type_error(format!(
                "{label}() argument {index} must be str, not {kind}"
            ))
        })
    }

    #[derive(FromArgs)]
    struct CodecEncodeArgs {
        #[pyarg(any)]
        input: PyObjectRef,
        #[pyarg(any, optional)]
        errors: OptionalOption<PyStrRef>,
    }

    #[derive(FromArgs)]
    struct CodecDecodeArgs {
        #[pyarg(any)]
        input: ArgBytesLike,
        #[pyarg(any, optional)]
        errors: OptionalOption<PyStrRef>,
    }

    #[derive(FromArgs)]
    struct IncrementalEncodeArgs {
        #[pyarg(any)]
        input: PyObjectRef,
        #[pyarg(any, optional, name = "final")]
        final_input: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct IncrementalDecodeArgs {
        #[pyarg(any)]
        input: ArgBytesLike,
        #[pyarg(any, optional, name = "final")]
        final_input: OptionalArg<PyObjectRef>,
    }

    /// The `bool(accept={int})` conversion the `final` arguments share, which
    /// takes any object and can run `__bool__`.
    fn final_arg(final_input: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<bool> {
        match final_input {
            OptionalArg::Present(obj) => obj.try_to_bool(vm),
            OptionalArg::Missing => Ok(false),
        }
    }

    #[pyattr]
    #[pyclass(name = "MultibyteIncrementalEncoder")]
    #[derive(Debug, PyPayload)]
    struct MultibyteIncrementalEncoder {
        codec: CodecRef,
        inner: PyMutex<EncoderState>,
    }

    impl Constructor for MultibyteIncrementalEncoder {
        type Args = StatefulArgs;

        fn py_new(cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let codec = CodecRef::from_class(cls, vm)?;
            let errors = errors_arg(args.errors, "IncrementalEncoder", 1, vm)?;
            let inner = codec.encoder_state(errors);
            Ok(Self {
                codec,
                inner: PyMutex::new(inner),
            })
        }
    }

    /// `mbiencoder_init` and friends: `__new__` did the work, so the
    /// pure-Python `__init__` further along the MRO must not run.
    impl Initializer for MultibyteIncrementalEncoder {
        type Args = FuncArgs;

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            Ok(())
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE))]
    impl MultibyteIncrementalEncoder {
        #[pygetset]
        fn errors(&self, vm: &VirtualMachine) -> PyStrRef {
            self.inner.lock().errors.name(vm)
        }

        #[pygetset(setter)]
        fn set_errors(
            &self,
            value: PySetterValue<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.inner.lock().errors = set_errors(value, vm)?;
            Ok(())
        }

        #[pymethod]
        fn encode(
            &self,
            args: IncrementalEncodeArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let final_input = final_arg(args.final_input, vm)?;
            // Driven on a detached copy: an error handler can re-enter this object.
            let mut local = self.inner.lock().clone();
            let result = self
                .codec
                .encode_stateful(&mut local, args.input, final_input, vm);
            self.commit(local);
            Ok(vm.ctx.new_bytes(result?).into())
        }

        fn commit(&self, local: EncoderState) {
            let mut inner = self.inner.lock();
            inner.state = local.state;
            inner.pending = local.pending;
        }

        #[pymethod]
        fn getstate(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
            let inner = self.inner.lock();
            let mut statebytes = Vec::with_capacity(ENCODER_STATE_SIZE);
            match &inner.pending {
                Some(pending) => {
                    let bytes = pending.as_wtf8().as_bytes();
                    if bytes.len() > MAXENCPENDING * 4 {
                        return Err(vm.new_unicode_encode_error_real(
                            vm.ctx.new_str(self.codec.encoding),
                            pending.clone(),
                            0,
                            pending.char_len(),
                            vm.ctx.new_str("pending buffer too large"),
                        ));
                    }
                    statebytes.push(bytes.len() as u8);
                    statebytes.extend_from_slice(bytes);
                }
                None => statebytes.push(0),
            }
            statebytes.extend_from_slice(&inner.state);
            Ok(state_to_int(&statebytes))
        }

        #[pymethod]
        fn setstate(&self, state: PyRef<PyInt>, vm: &VirtualMachine) -> PyResult<()> {
            let statebytes = int_to_state(&state, ENCODER_STATE_SIZE, vm)?;
            let pending_size = statebytes[0] as usize;
            if pending_size > MAXENCPENDING * 4 {
                return Err(vm.new_exception_msg(
                    vm.ctx.exceptions.unicode_error.to_owned(),
                    "pending buffer too large".into(),
                ));
            }
            let pending = vm.state.codec_registry.decode_text(
                vm.ctx
                    .new_bytes(statebytes[1..=pending_size].to_vec())
                    .into(),
                "utf-8",
                None,
                vm,
            )?;

            let mut inner = self.inner.lock();
            inner.pending = (!pending.is_empty()).then_some(pending);
            inner
                .state
                .copy_from_slice(&statebytes[1 + pending_size..9 + pending_size]);
            Ok(())
        }

        #[pymethod]
        fn reset(&self) {
            let mut inner = self.inner.lock();
            let _ = cjk::encode_reset(self.codec.codec, &mut inner.state);
            inner.pending = None;
        }
    }

    #[pyattr]
    #[pyclass(name = "MultibyteIncrementalDecoder")]
    #[derive(Debug, PyPayload)]
    struct MultibyteIncrementalDecoder {
        codec: CodecRef,
        inner: PyMutex<DecoderState>,
    }

    impl Constructor for MultibyteIncrementalDecoder {
        type Args = StatefulArgs;

        fn py_new(cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let codec = CodecRef::from_class(cls, vm)?;
            let errors = errors_arg(args.errors, "IncrementalDecoder", 1, vm)?;
            let inner = codec.decoder_state(errors);
            Ok(Self {
                codec,
                inner: PyMutex::new(inner),
            })
        }
    }

    /// `mbiencoder_init` and friends: `__new__` did the work, so the
    /// pure-Python `__init__` further along the MRO must not run.
    impl Initializer for MultibyteIncrementalDecoder {
        type Args = FuncArgs;

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            Ok(())
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE))]
    impl MultibyteIncrementalDecoder {
        #[pygetset]
        fn errors(&self, vm: &VirtualMachine) -> PyStrRef {
            self.inner.lock().errors.name(vm)
        }

        #[pygetset(setter)]
        fn set_errors(
            &self,
            value: PySetterValue<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.inner.lock().errors = set_errors(value, vm)?;
            Ok(())
        }

        #[pymethod]
        fn decode(&self, args: IncrementalDecodeArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let final_input = final_arg(args.final_input, vm)?;
            // Driven on a detached copy: an error handler can re-enter this object.
            let mut local = self.inner.lock().clone();
            let original_pending = core::mem::take(&mut local.pending);
            let mut data = original_pending.clone();
            data.extend_from_slice(&args.input.borrow_buf());
            let mut buf = DecodeBuffer {
                data,
                pos: 0,
                out: Wtf8Buf::new(),
                exception: None,
            };

            let errors = local.errors.clone();
            let mut error = None;
            if let Err(e) = decode_into(self.codec, &mut local.state, &mut buf, &errors, true, vm) {
                error = Some(e);
            } else if final_input
                && buf.pos < buf.data.len()
                && let Err(e) = decode_incomplete(self.codec.encoding, &mut buf, &errors, vm)
            {
                // Only the final flush hands the caller its pending bytes back.
                local.pending = original_pending;
                error = Some(e);
            } else if buf.pos < buf.data.len()
                && let Err(e) = self.codec.append_pending(&mut local, &buf, vm)
            {
                error = Some(e);
            }

            {
                let mut inner = self.inner.lock();
                inner.state = local.state;
                inner.pending = local.pending;
            }
            match error {
                Some(e) => Err(e),
                None => Ok(vm.ctx.new_str(buf.out)),
            }
        }

        #[pymethod]
        fn getstate(&self, vm: &VirtualMachine) -> (PyObjectRef, BigInt) {
            let inner = self.inner.lock();
            (
                vm.ctx.new_bytes(inner.pending.clone()).into(),
                state_to_int(&inner.state),
            )
        }

        #[pymethod]
        fn setstate(&self, state: PyRef<PyTuple>, vm: &VirtualMachine) -> PyResult<()> {
            let [buffer, flags] = state.as_slice() else {
                return Err(vm.new_type_error("setstate(): illegal state argument"));
            };
            let buffer = buffer
                .downcast_ref::<PyBytes>()
                .ok_or_else(|| vm.new_type_error("setstate(): illegal state argument"))?;
            let flags = flags
                .downcast_ref::<PyInt>()
                .ok_or_else(|| vm.new_type_error("setstate(): illegal state argument"))?;
            let statebytes = int_to_state(flags, 8, vm)?;

            let pending = buffer.as_bytes();
            if pending.len() > MAXDECPENDING {
                return Err(vm.new_unicode_decode_error(
                    vm.ctx.new_str(self.codec.encoding),
                    buffer.to_owned(),
                    0,
                    pending.len(),
                    vm.ctx.new_str("pending buffer too large"),
                ));
            }

            let mut inner = self.inner.lock();
            inner.pending = pending.to_vec();
            inner.state.copy_from_slice(&statebytes);
            Ok(())
        }

        #[pymethod]
        fn reset(&self) {
            let mut inner = self.inner.lock();
            cjk::decode_reset(self.codec.codec, &mut inner.state);
            inner.pending.clear();
        }
    }

    #[pyattr]
    #[pyclass(name = "MultibyteStreamReader", traverse)]
    #[derive(Debug, PyPayload)]
    struct MultibyteStreamReader {
        #[pytraverse(skip)]
        codec: CodecRef,
        stream: PyObjectRef,
        #[pytraverse(skip)]
        inner: PyMutex<DecoderState>,
    }

    impl Constructor for MultibyteStreamReader {
        type Args = StreamArgs;

        fn py_new(cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let codec = CodecRef::from_class(cls, vm)?;
            let errors = errors_arg(args.errors, "StreamReader", 2, vm)?;
            let inner = codec.decoder_state(errors);
            Ok(Self {
                codec,
                stream: args.stream,
                inner: PyMutex::new(inner),
            })
        }
    }

    /// `mbiencoder_init` and friends: `__new__` did the work, so the
    /// pure-Python `__init__` further along the MRO must not run.
    impl Initializer for MultibyteStreamReader {
        type Args = FuncArgs;

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            Ok(())
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE))]
    impl MultibyteStreamReader {
        #[pygetset]
        fn stream(&self) -> PyObjectRef {
            self.stream.clone()
        }

        #[pygetset]
        fn errors(&self, vm: &VirtualMachine) -> PyStrRef {
            self.inner.lock().errors.name(vm)
        }

        #[pygetset(setter)]
        fn set_errors(
            &self,
            value: PySetterValue<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.inner.lock().errors = set_errors(value, vm)?;
            Ok(())
        }

        /// `mbstreamreader_iread`.
        fn iread(&self, method: &str, sizehint: isize, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            if sizehint == 0 {
                return Ok(Wtf8Buf::new());
            }
            // Driven on a detached copy: the stream and the error handler are
            // both Python code that can re-enter this reader.
            let mut local = self.inner.lock().clone();
            let mut out = Wtf8Buf::new();
            let result = self.iread_into(method, sizehint, &mut local, &mut out, vm);
            {
                let mut inner = self.inner.lock();
                inner.state = local.state;
                inner.pending = local.pending;
            }
            result.map(|()| out)
        }

        fn iread_into(
            &self,
            method: &str,
            sizehint: isize,
            local: &mut DecoderState,
            out: &mut Wtf8Buf,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let errors = local.errors.clone();
            let mut sizehint = sizehint;
            loop {
                let read = if sizehint < 0 {
                    vm.call_method(&self.stream, method, ())?
                } else {
                    vm.call_method(&self.stream, method, (sizehint,))?
                };
                let read = read.downcast::<PyBytes>().map_err(|read| {
                    vm.new_type_error(format!(
                        "stream function returned a non-bytes object ({})",
                        read.class().name()
                    ))
                })?;
                let end_of_file = read.as_bytes().is_empty();

                let mut data = core::mem::take(&mut local.pending);
                data.extend_from_slice(read.as_bytes());
                let size = data.len();
                let mut buf = DecodeBuffer {
                    data,
                    pos: 0,
                    out: core::mem::take(out),
                    exception: None,
                };
                let decoded = (|| {
                    if size > 0 {
                        decode_into(self.codec, &mut local.state, &mut buf, &errors, true, vm)?;
                    }
                    if (end_of_file || sizehint < 0) && buf.pos < buf.data.len() {
                        decode_incomplete(self.codec.encoding, &mut buf, &errors, vm)?;
                    }
                    if buf.pos < buf.data.len() {
                        self.codec.append_pending(local, &buf, vm)?;
                    }
                    Ok(())
                })();
                *out = buf.out;
                decoded?;

                if sizehint < 0 || !out.is_empty() || read.as_bytes().is_empty() {
                    break;
                }
                // Read one more byte and retry.
                sizehint = 1;
            }
            Ok(())
        }

        #[pymethod]
        fn read(&self, size: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let size = size_hint(size, vm)?;
            Ok(vm.ctx.new_str(self.iread("read", size, vm)?))
        }

        #[pymethod]
        fn readline(
            &self,
            size: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyStrRef> {
            let size = size_hint(size, vm)?;
            Ok(vm.ctx.new_str(self.iread("readline", size, vm)?))
        }

        #[pymethod]
        fn readlines(&self, size: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
            let size = size_hint(size, vm)?;
            let text = vm.ctx.new_str(self.iread("read", size, vm)?);
            vm.call_method(text.as_object(), "splitlines", (true,))
        }

        #[pymethod]
        fn reset(&self) {
            let mut inner = self.inner.lock();
            cjk::decode_reset(self.codec.codec, &mut inner.state);
            inner.pending.clear();
        }
    }

    #[pyattr]
    #[pyclass(name = "MultibyteStreamWriter", traverse)]
    #[derive(Debug, PyPayload)]
    struct MultibyteStreamWriter {
        #[pytraverse(skip)]
        codec: CodecRef,
        stream: PyObjectRef,
        #[pytraverse(skip)]
        inner: PyMutex<EncoderState>,
    }

    impl Constructor for MultibyteStreamWriter {
        type Args = StreamArgs;

        fn py_new(cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let codec = CodecRef::from_class(cls, vm)?;
            let errors = errors_arg(args.errors, "StreamWriter", 2, vm)?;
            let inner = codec.encoder_state(errors);
            Ok(Self {
                codec,
                stream: args.stream,
                inner: PyMutex::new(inner),
            })
        }
    }

    /// `mbiencoder_init` and friends: `__new__` did the work, so the
    /// pure-Python `__init__` further along the MRO must not run.
    impl Initializer for MultibyteStreamWriter {
        type Args = FuncArgs;

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            Ok(())
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE))]
    impl MultibyteStreamWriter {
        #[pygetset]
        fn stream(&self) -> PyObjectRef {
            self.stream.clone()
        }

        #[pygetset]
        fn errors(&self, vm: &VirtualMachine) -> PyStrRef {
            self.inner.lock().errors.name(vm)
        }

        #[pygetset(setter)]
        fn set_errors(
            &self,
            value: PySetterValue<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.inner.lock().errors = set_errors(value, vm)?;
            Ok(())
        }

        /// `mbstreamwriter_iwrite`.
        fn iwrite(&self, text: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // Driven on a detached copy: an error handler can re-enter this writer.
            let mut local = self.inner.lock().clone();
            let result = self.codec.encode_stateful(&mut local, text, false, vm);
            self.commit(local);
            let encoded = result?;
            vm.call_method(&self.stream, "write", (vm.ctx.new_bytes(encoded),))?;
            Ok(())
        }

        fn commit(&self, local: EncoderState) {
            let mut inner = self.inner.lock();
            inner.state = local.state;
            inner.pending = local.pending;
        }

        #[pymethod]
        fn write(&self, text: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.iwrite(text, vm)
        }

        #[pymethod]
        fn writelines(&self, lines: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let sequence = PySequence { obj: &lines };
            if !sequence.check() {
                return Err(vm.new_type_error("arg must be a sequence object"));
            }
            let mut i = 0;
            while i < sequence.length(vm)? {
                let line = sequence.get_item(i as isize, vm)?;
                self.iwrite(line, vm)?;
                i += 1;
            }
            Ok(())
        }

        #[pymethod]
        fn reset(&self, vm: &VirtualMachine) -> PyResult<()> {
            let mut local = self.inner.lock().clone();
            let Some(pending) = local.pending.take() else {
                return Ok(());
            };
            let errors = local.errors.clone();
            // A strict failure drops the pending text: reset exists to clear it.
            let result = encode(
                self.codec,
                &mut local.state,
                pending,
                &errors,
                true,
                true,
                vm,
            );
            self.commit(local);
            let encoded = result?.0;
            if !encoded.is_empty() {
                vm.call_method(&self.stream, "write", (vm.ctx.new_bytes(encoded),))?;
            }
            Ok(())
        }
    }

    /// `codecctx_errors_set`.
    fn set_errors(
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<ErrorHandler> {
        let PySetterValue::Assign(value) = value else {
            return Err(vm.new_attribute_error("cannot delete attribute"));
        };
        let value = value
            .downcast::<PyStr>()
            .map_err(|_| vm.new_type_error("errors must be a string"))?;
        Ok(ErrorHandler::new(Some(value)))
    }

    /// The `sizeobj` conversion the stream reader's methods share.
    fn size_hint(size: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<isize> {
        let Some(size) = size.into_option() else {
            return Ok(-1);
        };
        if vm.is_none(&size) {
            return Ok(-1);
        }
        let size = size
            .downcast_ref::<PyInt>()
            .ok_or_else(|| vm.new_type_error("arg 1 must be an integer"))?;
        size.try_to_primitive::<isize>(vm)
    }
}
