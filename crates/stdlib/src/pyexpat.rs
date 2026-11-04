//! Pyexpat builtin module
//!
//! ## Incremental parsing / threading protocol
//!
//! `xml.etree.ElementTree`, `xml.sax`, and similar callers feed a document to
//! `Parse()`/`ParseFile()` in many small chunks (e.g. 64KB at a time) rather
//! than all at once, and expect the parser to retain state across those
//! calls the way libexpat's `XML_Parse` does. `xml-rs`'s `EventReader` has no
//! API to pause and resume across separate `Read` sources, so two backends
//! are provided (`Backend::Threaded`/`Backend::Sync`), selected per-parser
//! the first time it is fed (`PyExpatLikeXmlParser::feed_inner`):
//!
//! - `Backend::Threaded` (`ParserStream`, used wherever an OS thread can be
//!   spawned) drives one `EventReader` to completion on a dedicated
//!   background thread, fed through a `Read` implementation (`FeedReader`)
//!   backed by a shared byte queue (`FeedBuffer`). This keeps the
//!   per-`Parse()` cost proportional to the size of that call's chunk,
//!   rather than to the whole document seen so far. See the protocol notes
//!   below.
//! - `Backend::Sync` (`SyncParser`) is used on targets that cannot spawn
//!   threads (currently `wasm32`), or if spawning fails at runtime. It
//!   appends every chunk to a persistent buffer and re-parses the buffer
//!   from scratch on each `Parse()`/`ParseFile()` call, replaying only the
//!   events at or beyond the count already dispatched by previous calls.
//!   This is correct across chunk boundaries but costs O(total bytes fed so
//!   far) per call, so it is only used where the threaded backend is
//!   unavailable.
//!
//! Both backends share `LineTracker` to convert `xml-rs`'s row/column
//! `TextPosition` into `CurrentLineNumber`/`CurrentColumnNumber`/
//! `CurrentByteIndex`, so the two backends report identical positions and
//! identical handler/error semantics.
//!
//! `Backend::Threaded` protocol, implemented by
//! `ParserStream`/`FeedBuffer`/`run_parser_thread`:
//! - `ParserCreate()` does not spawn a thread; one is spawned lazily by the
//!   first `Parse()`/`ParseFile()` call, so a parser created and dropped
//!   without being fed leaks nothing.
//! - `Parse(data, isfinal)` pushes `data` into the `FeedBuffer` (closing it
//!   if `isfinal`), wakes the parser thread via a `Condvar`, then drains
//!   (`pump`) the message channel the parser thread reports events through,
//!   dispatching each to its handler *on the calling thread* -- the parser
//!   thread itself never touches the VM or calls into Python.
//! - When the parser thread's `Read::read` finds the buffer empty and not
//!   yet closed, it sends `ParserMsg::NeedMore` and blocks on the `Condvar`.
//!   Seeing `NeedMore` is how the calling thread knows the parser thread has
//!   consumed everything fed so far and it is safe to return control to
//!   Python; `ParserMsg::Finished`/`Error` mean the thread has exited.
//! - If a handler raises, `pump` stops dispatching and propagates the error
//!   immediately, but leaves the parser thread and channel alone (any
//!   already-produced events stay queued) so the *next* `Parse()` call
//!   resumes exactly where dispatch left off, mirroring libexpat (a callback
//!   exception aborts that `XML_Parse()` call without corrupting the
//!   parser's internal state).
//! - Calling `Parse()`/`ParseFile()` reentrantly (e.g. from within a
//!   handler) would deadlock waiting on a channel that same call is meant to
//!   drain, so it is rejected with a `RuntimeError` via a `busy` flag
//!   instead (shared with `Backend::Sync`).
//! - Dropping the parser (`ParserStream::drop`) always stops (`FeedBuffer`
//!   is marked stopped, waking any blocked `read()`) and joins the thread,
//!   so no thread is ever leaked, whether or not the parser was ever fed to
//!   completion.
//! - If spawning the background thread fails at runtime (e.g. the platform
//!   claims thread support but is out of resources), `Backend::new` falls
//!   back to `Backend::Sync` for that parser instead of panicking.
//!
//! ## Fork safety
//!
//! `fork()` only ever keeps the calling thread; every other OS thread the
//! parent had simply vanishes from the child's point of view, without
//! running its destructors or releasing whatever it held. Two hazards
//! follow for `Backend::Threaded`, both observed to crash the child (not
//! just theoretical UB) when exercised (e.g. `platform.mac_ver()`, which
//! parses a plist through `plistlib`/`pyexpat`, called once before
//! `os.fork()` and again in the child):
//! - A `ParserStream` inherited from before the fork (kept alive past the
//!   fork by a reference cycle, e.g. plistlib's parser/handler cycle, until
//!   GC runs) may have its background thread gone but its `FeedBuffer`
//!   mutex/condvar and channel still exist. If that thread was holding one
//!   of those locks (or blocked in `Condvar::wait`, which reacquires the
//!   mutex internally) at the instant of `fork()`, the lock looks "held"
//!   forever in the child, and even *destroying* a POSIX mutex that looks
//!   held is undefined behavior, not just locking it.
//! - Spawning a *brand new* parser thread from the child -- the common case,
//!   e.g. a fresh `ParserCreate()` used for the first time after `fork()`
//!   -- was empirically found to reliably crash the process during the new
//!   thread's startup, even with no inherited `ParserStream` involved at
//!   all (confirmed by bisecting away every other explanation: a plain
//!   Python-level `threading.Thread` survives the same fork fine, since it
//!   goes through this VM's own thread bookkeeping, which
//!   `stop_the_world`/`reinit_after_fork` (see `crates/vm/src/vm/mod.rs`,
//!   `crates/vm/src/stdlib/posix.rs`) already make fork-safe; a raw
//!   `std::thread::spawn` bypasses all of that).
//!
//! Both are handled the same way: a `pthread_atfork` child hook (registered
//! once, lazily, the first time a parser thread is spawned) sets a
//! process-wide, permanently-sticky flag the instant this process is ever
//! the child of a `fork()`. Once set:
//! - `ParserStream::try_spawn` refuses to spawn a new thread at all (falls
//!   back to `Backend::Sync`, exactly as on targets that cannot spawn
//!   threads in the first place), which is what actually avoids the second
//!   hazard above -- a *fresh* parser created in the child still works,
//!   just via the slower backend.
//! - Feeding an inherited `Backend::Threaded` raises a clear `ExpatError`
//!   instead of touching its `FeedBuffer`/channel, and dropping it (from
//!   GC, in `ParserStream::drop`) leaks its `JoinHandle`/buffer/channel
//!   rather than joining or destroying them -- one abandoned parser thread
//!   per fork-with-a-parser-mid-flight is a bounded, one-time cost, not a
//!   live hazard, and is exactly the `mem::forget`-the-handle approach
//!   `Backend::Sync` never needs because it never owns a thread.
//!
//! The parent process is never affected: the `pthread_atfork` *child* hook
//! only ever runs inside the child, so the flag (and therefore this whole
//! code path) stays off for the parent's own parsers.

// spell-checker: ignore libexpat

pub(crate) use _pyexpat::module_def;

macro_rules! create_property {
    ($ctx: expr, $attributes: expr, $name: expr, $class: expr, $element: ident) => {
        let attr = $ctx.new_static_getset(
            $name,
            $class,
            move |this: &Py<PyExpatLikeXmlParser>| this.$element.read().clone(),
            move |this: &Py<PyExpatLikeXmlParser>, func: PyObjectRef| *this.$element.write() = func,
        );

        $attributes.insert($ctx.intern_str($name), attr.into());
    };
}

macro_rules! create_readonly_int_property {
    ($ctx: expr, $attributes: expr, $name: expr, $class: expr, $element: ident) => {
        let getset = crate::vm::builtins::PyGetSet::new($name, $class, $ctx).with_get(
            move |this: &Py<PyExpatLikeXmlParser>, vm: &VirtualMachine| -> PyObjectRef {
                vm.ctx.new_int(*this.$element.read()).into()
            },
        );
        let attr = crate::vm::PyRef::new_ref(getset, $ctx.types.getset_type.to_owned(), None);

        $attributes.insert($ctx.intern_str($name), attr.into());
    };
}

macro_rules! create_bool_property {
    ($ctx: expr, $attributes: expr, $name: expr, $class: expr, $element: ident) => {
        let attr = $ctx.new_static_getset(
            $name,
            $class,
            move |this: &Py<PyExpatLikeXmlParser>| this.$element.read().clone(),
            move |this: &Py<PyExpatLikeXmlParser>,
                  value: PyObjectRef,
                  vm: &VirtualMachine|
                  -> PyResult<()> {
                let bool_value = value.is_true(vm)?;
                *this.$element.write() = vm.ctx.new_bool(bool_value).into();
                Ok(())
            },
        );

        $attributes.insert($ctx.intern_str($name), attr.into());
    };
}

#[pymodule(name = "pyexpat")]
mod _pyexpat {
    use crate::vm::{
        AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        VirtualMachine,
        builtins::{PyException, PyModule, PyStr, PyStrRef, PyType, PyUtf8StrRef},
        extend_module,
        function::{
            ArgBytesLike, ArgIndex, ArgStrOrBytesLike, FuncArgs, IntoFuncArgs, OptionalArg,
            OptionalOption,
        },
        types::Constructor,
    };
    use alloc::collections::VecDeque;
    use alloc::rc::Rc;
    use core::cell::RefCell;
    use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use rustpython_common::lock::PyRwLock;
    use std::io::Read;
    use xml::common::Position;
    use xml::reader::XmlEvent;

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);

        // Add submodules
        let model = super::_model::module_def(&vm.ctx).create_module(vm)?;
        let errors_def = super::_errors::module_def(&vm.ctx);
        let errors = errors_def.create_module(vm)?;
        errors_def.exec_module(vm, &errors)?;
        vm.sys_module.get_attr("modules", vm)?.set_item(
            "pyexpat.errors",
            errors.clone().into(),
            vm,
        )?;

        extend_module!(vm, module, {
            "model" => model,
            "errors" => errors,
        });

        Ok(())
    }

    #[pyfunction(name = "ErrorString")]
    fn error_string(code: ArgIndex, vm: &VirtualMachine) -> PyResult<Option<&'static str>> {
        let code = code
            .into_int_ref()
            .try_to_primitive::<core::ffi::c_long>(vm)?;
        // CPython converts a C long to int before calling XML_ErrorString.
        let index = (code as core::ffi::c_int).wrapping_sub(1) as usize;
        Ok(super::_errors::ERRORS
            .get(index)
            .map(|(_, message)| *message))
    }

    type MutableObject = PyRwLock<PyObjectRef>;

    #[pyattr(name = "version_info")]
    pub(super) const VERSION_INFO: (u32, u32, u32) = (2, 7, 1);

    #[pyattr]
    const XML_PARAM_ENTITY_PARSING_NEVER: i32 = 0;
    #[pyattr]
    const XML_PARAM_ENTITY_PARSING_UNLESS_STANDALONE: i32 = 1;
    #[pyattr]
    const XML_PARAM_ENTITY_PARSING_ALWAYS: i32 = 2;

    #[pyattr]
    #[pyattr(name = "XMLParserType")]
    #[pyclass(name = "xmlparser", traverse)]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyExpatLikeXmlParser {
        #[pytraverse(skip)]
        namespace_separator: Option<String>,
        #[pytraverse(skip)]
        base: PyRwLock<Option<String>>,
        // Position of the last event produced, exposed as CurrentLineNumber /
        // CurrentColumnNumber / CurrentByteIndex (mirrors libexpat's
        // XML_GetCurrent* family). Updated before each handler invocation.
        #[pytraverse(skip)]
        current_line: PyRwLock<i64>,
        #[pytraverse(skip)]
        current_column: PyRwLock<i64>,
        #[pytraverse(skip)]
        current_byte_index: PyRwLock<i64>,
        // Incremental-parsing state: Parse()/ParseFile() may be called
        // multiple times with successive chunks of the same document (e.g.
        // xml.etree.ElementTree and genshi both feed input in ~64KB chunks).
        // See the module-level `//!` doc comment for the full
        // threaded/sync-backend protocol; `backend` is `None` until the
        // first `Parse()`/`ParseFile()` call, so a parser that is created
        // and dropped without ever being fed never spawns a thread or
        // allocates a buffer.
        #[pytraverse(skip)]
        backend: PyRwLock<Option<Backend>>,
        // Guards against Parse()/ParseFile() being invoked reentrantly (e.g.
        // a handler calling Parse() on its own parser), which would
        // otherwise deadlock waiting on a channel this same call is meant to
        // drain.
        #[pytraverse(skip)]
        busy: AtomicBool,
        // Prevent creating another backend after finalization or fork failure.
        #[pytraverse(skip)]
        finished: AtomicBool,
        // Distinguish successful completion from the existing failure paths.
        #[pytraverse(skip)]
        completed: AtomicBool,
        #[pytraverse(skip)]
        namespace_stack: PyRwLock<Vec<(xml::namespace::Namespace, Vec<String>)>>,
        #[pytraverse(skip)]
        external_entities: PyRwLock<ExternalEntityMap>,
        #[pytraverse(skip)]
        reparse_deferral_enabled: AtomicBool,
        #[pytraverse(skip)]
        parameter_entity_parsing: AtomicI32,
        #[pytraverse(skip)]
        use_foreign_dtd: AtomicBool,
        #[pytraverse(skip)]
        standalone: AtomicBool,
        #[pytraverse(skip)]
        external_subset_handled: AtomicBool,
        #[pytraverse(skip)]
        started: AtomicBool,
        #[pytraverse(skip)]
        external_entity: Option<ExternalEntity>,
        parent: Option<PyExpatLikeXmlParserRef>,
        start_element: MutableObject,
        end_element: MutableObject,
        character_data: MutableObject,
        entity_decl: MutableObject,
        buffer_text: MutableObject,
        namespace_prefixes: MutableObject,
        ordered_attributes: MutableObject,
        specified_attributes: MutableObject,
        #[pymember]
        intern: PyObjectRef,
        // Additional handlers (stubs for compatibility)
        processing_instruction: MutableObject,
        unparsed_entity_decl: MutableObject,
        notation_decl: MutableObject,
        start_namespace_decl: MutableObject,
        end_namespace_decl: MutableObject,
        comment: MutableObject,
        start_cdata_section: MutableObject,
        end_cdata_section: MutableObject,
        default: MutableObject,
        default_expand: MutableObject,
        not_standalone: MutableObject,
        external_entity_ref: MutableObject,
        start_doctype_decl: MutableObject,
        end_doctype_decl: MutableObject,
        xml_decl: MutableObject,
        element_decl: MutableObject,
        attlist_decl: MutableObject,
        skipped_entity: MutableObject,
    }
    type PyExpatLikeXmlParserRef = PyRef<PyExpatLikeXmlParser>;

    #[inline]
    fn invoke_handler<T>(vm: &VirtualMachine, handler: &MutableObject, args: T) -> PyResult<()>
    where
        T: IntoFuncArgs,
    {
        // Clone the handler while holding the read lock, then release the lock
        let handler = handler.read().clone();
        if vm.is_none(&handler) {
            return Ok(());
        }
        // Mirrors libexpat/CPython: an exception raised from a handler aborts
        // parsing and propagates out of Parse()/ParseFile(), instead of being
        // silently discarded.
        handler.call(args, vm)?;
        Ok(())
    }

    fn invoke_handler_args(
        vm: &VirtualMachine,
        handler: &MutableObject,
        args: Vec<PyObjectRef>,
    ) -> PyResult<()> {
        let handler = handler.read().clone();
        if vm.is_none(&handler) {
            return Ok(());
        }
        handler.call(FuncArgs::from(args), vm)?;
        Ok(())
    }

    fn invoke_int_handler<T>(
        vm: &VirtualMachine,
        handler: &MutableObject,
        name: &str,
        args: T,
    ) -> PyResult<Option<i32>>
    where
        T: IntoFuncArgs,
    {
        let handler = handler.read().clone();
        if vm.is_none(&handler) {
            return Ok(None);
        }
        let result = handler.call(args, vm)?;
        core::ffi::c_long::try_from_object(vm, result)
            .map(|value| {
                // Expat callbacks convert C long to int. These types have the
                // same width on Windows, but differ on 64-bit Unix targets.
                #[allow(clippy::unnecessary_cast)]
                let value = value as i32;
                Some(value)
            })
            .inspect_err(|error| {
                let _ = error.add_note(
                    vm.ctx
                        .new_str(format!("invalid '{name}' event handler return value")),
                    vm,
                );
            })
    }

    #[derive(Debug)]
    struct ExternalEntity {
        context: Option<String>,
        encoding: Option<String>,
    }

    type ExternalEntityIds = (Option<String>, Option<String>);
    type ExternalEntityMap = alloc::collections::BTreeMap<String, ExternalEntityIds>;

    struct Doctype {
        syntax: String,
        name: String,
        system_id: Option<String>,
        public_id: Option<String>,
        has_internal_subset: bool,
        has_parameter_entity_reference: bool,
        skipped_parameter_entities: Vec<String>,
    }

    enum DtdDeclaration {
        Comment(String),
        ProcessingInstruction(String, String),
        Element(String, String),
        Attlist(String, String, String, Option<String>, bool),
        Notation(String, Option<String>, Option<String>),
        Entity {
            name: String,
            parameter: bool,
            value: Option<String>,
            system_id: Option<String>,
            public_id: Option<String>,
            notation: Option<String>,
        },
    }

    fn normalize_public_id(value: &str) -> String {
        let mut normalized = String::with_capacity(value.len());
        let mut pending_space = false;
        for character in value.chars() {
            if matches!(character, ' ' | '\t' | '\r' | '\n') {
                pending_space = !normalized.is_empty();
            } else {
                if pending_space {
                    normalized.push(' ');
                    pending_space = false;
                }
                normalized.push(character);
            }
        }
        normalized
    }

    fn decode_xml_entity_value(value: &str) -> String {
        let mut output = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(start) = rest.find("&#") {
            output.push_str(&rest[..start]);
            let after_start = &rest[start + 2..];
            let Some(end) = after_start.find(';') else {
                output.push_str(&rest[start..]);
                return output;
            };
            let number = &after_start[..end];
            let decoded = if let Some(digits) = number
                .strip_prefix('x')
                .or_else(|| number.strip_prefix('X'))
            {
                u32::from_str_radix(digits, 16).ok()
            } else {
                number.parse().ok()
            }
            .and_then(char::from_u32);
            if let Some(character) = decoded {
                output.push(character);
            } else {
                output.push_str(&rest[start..=(start + 2 + end)]);
            }
            rest = &after_start[end + 1..];
        }
        output.push_str(rest);
        output
    }

    fn normalize_attlist_default(value: &str, kind: &str) -> String {
        let decoded = decode_xml_entity_value(value);
        let replaced = decoded.replace(['\t', '\r', '\n'], " ");
        if kind == "CDATA" {
            replaced
        } else {
            replaced.split_whitespace().collect::<Vec<_>>().join(" ")
        }
    }

    fn dtd_declarations(syntax: &str) -> Vec<DtdDeclaration> {
        let Some((subset_start, subset_end)) = internal_subset_bounds(syntax) else {
            return Vec::new();
        };
        let subset = &syntax[subset_start + 1..subset_end];
        let mut declarations = Vec::new();
        let mut offset = 0;
        while let Some(relative) = subset[offset..].find('<') {
            let start = offset + relative;
            if subset[start..].starts_with("<!--") {
                let Some(end) = subset[start + 4..].find("-->") else {
                    break;
                };
                declarations.push(DtdDeclaration::Comment(
                    subset[start + 4..start + 4 + end].to_owned(),
                ));
                offset = start + 4 + end + 3;
                continue;
            }
            if subset[start..].starts_with("<?") {
                let Some(end) = subset[start + 2..].find("?>") else {
                    break;
                };
                let content = subset[start + 2..start + 2 + end].trim_start();
                let split = content.find(char::is_whitespace).unwrap_or(content.len());
                let target = content[..split].to_owned();
                let data = content[split..].to_owned();
                declarations.push(DtdDeclaration::ProcessingInstruction(target, data));
                offset = start + 2 + end + 2;
                continue;
            }
            if !subset[start..].starts_with("<!") {
                offset = start + 1;
                continue;
            }
            let rest = &subset[start + 2..];
            let mut quote = None;
            let Some(end) = rest.char_indices().find_map(|(index, character)| {
                if let Some(delimiter) = quote {
                    if character == delimiter {
                        quote = None;
                    }
                } else if matches!(character, '\'' | '"') {
                    quote = Some(character);
                } else if character == '>' {
                    return Some(index);
                }
                None
            }) else {
                break;
            };
            let declaration = rest[..end].trim();
            let mut tokens = Vec::new();
            let mut quoted_tokens = Vec::new();
            let mut token = String::new();
            let mut token_quote = None;
            let mut group_depth = 0;
            for character in declaration.chars() {
                if let Some(delimiter) = token_quote {
                    if character == delimiter {
                        token_quote = None;
                        tokens.push(core::mem::take(&mut token));
                        quoted_tokens.push(true);
                    } else {
                        token.push(character);
                    }
                } else if matches!(character, '\'' | '"') {
                    if !token.is_empty() {
                        tokens.push(core::mem::take(&mut token));
                        quoted_tokens.push(false);
                    }
                    token_quote = Some(character);
                } else if character == '(' {
                    group_depth += 1;
                    token.push(character);
                } else if character == ')' && group_depth > 0 {
                    group_depth -= 1;
                    token.push(character);
                } else if character.is_whitespace() {
                    if group_depth == 0 && !token.is_empty() {
                        tokens.push(core::mem::take(&mut token));
                        quoted_tokens.push(false);
                    }
                } else {
                    token.push(character);
                }
            }
            if !token.is_empty() {
                tokens.push(token);
                quoted_tokens.push(false);
            }
            match tokens.first().map(String::as_str) {
                Some("ELEMENT") if tokens.len() >= 3 => declarations.push(DtdDeclaration::Element(
                    tokens[1].clone(),
                    tokens[2].clone(),
                )),
                Some("ATTLIST") if tokens.len() >= 5 => {
                    let element = tokens[1].clone();
                    let mut index = 2;
                    while index + 2 < tokens.len() {
                        let name = tokens[index].clone();
                        let mut kind = tokens[index + 1].clone();
                        let mut default_index = index + 2;
                        if kind.starts_with('(') && !kind.ends_with(')') {
                            while default_index < tokens.len() {
                                kind.push_str(&tokens[default_index]);
                                default_index += 1;
                                if kind.ends_with(')') {
                                    break;
                                }
                            }
                        } else if kind == "NOTATION"
                            && tokens
                                .get(default_index)
                                .is_some_and(|v| v.starts_with('('))
                        {
                            kind.push_str(&tokens[default_index]);
                            default_index += 1;
                            while default_index < tokens.len() && !kind.ends_with(')') {
                                kind.push_str(&tokens[default_index]);
                                default_index += 1;
                            }
                        }
                        let Some(default_kind) = tokens.get(default_index) else {
                            break;
                        };
                        let default_is_quoted = quoted_tokens.get(default_index) == Some(&true);
                        let (default, required, next) = match default_kind.as_str() {
                            "#REQUIRED" if !default_is_quoted => (None, true, default_index + 1),
                            "#IMPLIED" if !default_is_quoted => (None, false, default_index + 1),
                            "#FIXED" if !default_is_quoted && default_index + 1 < tokens.len() => (
                                Some(tokens[default_index + 1].clone()),
                                true,
                                default_index + 2,
                            ),
                            value => (Some(value.to_owned()), false, default_index + 1),
                        };
                        declarations.push(DtdDeclaration::Attlist(
                            element.clone(),
                            name,
                            kind,
                            default,
                            required,
                        ));
                        index = next;
                    }
                }
                Some("NOTATION") if tokens.len() >= 4 => {
                    let name = tokens[1].clone();
                    let (public_id, system_id) = match tokens[2].as_str() {
                        "SYSTEM" => (None, Some(tokens[3].clone())),
                        "PUBLIC" => (
                            Some(normalize_public_id(&tokens[3])),
                            tokens
                                .get(4)
                                .filter(|value| !value.starts_with('>'))
                                .cloned(),
                        ),
                        _ => (None, None),
                    };
                    declarations.push(DtdDeclaration::Notation(name, public_id, system_id));
                }
                Some("ENTITY") if tokens.len() >= 3 => {
                    let parameter = tokens.get(1).is_some_and(|value| value == "%");
                    let name_index = if parameter { 2 } else { 1 };
                    let Some(name) = tokens.get(name_index).cloned() else {
                        offset = start + end + 3;
                        continue;
                    };
                    let value_index = name_index + 1;
                    if let Some(value) = tokens.get(value_index)
                        && (quoted_tokens.get(value_index) == Some(&true)
                            || !matches!(value.as_str(), "SYSTEM" | "PUBLIC"))
                    {
                        declarations.push(DtdDeclaration::Entity {
                            name,
                            parameter,
                            value: Some(value.clone()),
                            system_id: None,
                            public_id: None,
                            notation: None,
                        });
                    } else {
                        let (public_id, system_id, notation) =
                            match tokens.get(value_index).map(String::as_str) {
                                Some("SYSTEM") => (
                                    None,
                                    tokens.get(value_index + 1).cloned(),
                                    tokens
                                        .iter()
                                        .position(|token| token == "NDATA")
                                        .and_then(|position| tokens.get(position + 1).cloned()),
                                ),
                                Some("PUBLIC") => (
                                    tokens
                                        .get(value_index + 1)
                                        .map(|value| normalize_public_id(value)),
                                    tokens.get(value_index + 2).cloned(),
                                    tokens
                                        .iter()
                                        .position(|token| token == "NDATA")
                                        .and_then(|position| tokens.get(position + 1).cloned()),
                                ),
                                _ => (None, None, None),
                            };
                        declarations.push(DtdDeclaration::Entity {
                            name,
                            parameter,
                            value: None,
                            system_id,
                            public_id,
                            notation,
                        });
                    }
                }
                _ => {}
            }
            offset = start + end + 3;
        }
        declarations
    }

    fn internal_subset_bounds(syntax: &str) -> Option<(usize, usize)> {
        let mut quote = None;
        let mut open = None;
        let bytes = syntax.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if quote.is_none() && bytes[index..].starts_with(b"<!--") {
                index = bytes[index + 4..]
                    .windows(3)
                    .position(|window| window == b"-->")
                    .map_or(bytes.len(), |end| index + 4 + end + 3);
                continue;
            }
            if quote.is_none() && bytes[index..].starts_with(b"<?") {
                index = bytes[index + 2..]
                    .windows(2)
                    .position(|window| window == b"?>")
                    .map_or(bytes.len(), |end| index + 2 + end + 2);
                continue;
            }
            let character = syntax[index..].chars().next()?;
            if let Some(delimiter) = quote {
                if character == delimiter {
                    quote = None;
                }
            } else if matches!(character, '\'' | '"') {
                quote = Some(character);
            } else if character == '[' && open.is_none() {
                open = Some(index);
            } else if character == ']' && open.is_some() {
                return open.map(|start| (start, index));
            }
            index += character.len_utf8();
        }
        None
    }

    fn entity_marker(kind: char, name: &str) -> String {
        format!("\0{kind}{name}\0")
    }

    fn remove_skipped_entity_markers(value: &str) -> String {
        let mut output = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(start) = rest.find("\0S") {
            output.push_str(&rest[..start]);
            let marker = &rest[start + 2..];
            let Some(end) = marker.find('\0') else {
                output.push_str(&rest[start..]);
                return output;
            };
            rest = &marker[end + 1..];
        }
        output.push_str(rest);
        output
    }

    fn dtd_entity_replacements(
        syntax: &str,
        references: impl IntoIterator<Item = String>,
    ) -> Vec<(String, String)> {
        let declarations = dtd_declarations(syntax);
        let external_subset =
            internal_subset_bounds(syntax).map_or(syntax, |(start, _)| &syntax[..start]);
        let may_have_external_entities = external_subset_id(external_subset);
        let mut declared = std::collections::HashSet::new();
        let mut replacements = alloc::collections::BTreeMap::new();
        let mut references: Vec<String> = references.into_iter().collect();
        for declaration in &declarations {
            match declaration {
                DtdDeclaration::Entity {
                    name,
                    parameter,
                    value,
                    system_id,
                    notation,
                    ..
                } => {
                    let first_declaration = *parameter || declared.insert(name.clone());
                    if !*parameter
                        && first_declaration
                        && value.is_none()
                        && notation.is_none()
                        && system_id.is_some()
                    {
                        replacements.insert(name.clone(), entity_marker('E', name));
                    }
                    if let Some(value) = value {
                        references.extend(named_entity_references(value));
                    }
                }
                DtdDeclaration::Attlist(_, _, kind, default, _) => {
                    if let Some(default) = default {
                        references.extend(named_entity_references(default));
                    }
                    references.extend(named_entity_references(kind));
                }
                _ => {}
            }
        }
        for name in references {
            if matches!(name.as_str(), "amp" | "lt" | "gt" | "apos" | "quot")
                || name.starts_with('#')
                || declared.contains(&name)
            {
                continue;
            }
            if may_have_external_entities {
                replacements
                    .entry(name.clone())
                    .or_insert_with(|| entity_marker('S', &name));
            }
        }
        replacements.into_iter().collect()
    }

    fn named_entity_references(value: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = value;
        while let Some(start) = rest.find('&') {
            rest = &rest[start + 1..];
            let Some(end) = rest.find(';') else {
                break;
            };
            let name = &rest[..end];
            if !name.is_empty() {
                names.push(name.to_owned());
            }
            rest = &rest[end + 1..];
        }
        names
    }

    fn decode_scanned_xml_name(bytes: &[u8], encoding: Option<&str>) -> String {
        if encoding.is_some_and(|encoding| {
            matches!(
                encoding.to_ascii_lowercase().replace('_', "-").as_str(),
                "iso-8859-1" | "latin1" | "latin-1"
            )
        }) {
            bytes.iter().map(|byte| char::from(*byte)).collect()
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        }
    }

    fn external_subset_id(header: &str) -> bool {
        let Some(header) = header.strip_prefix("<!DOCTYPE") else {
            return false;
        };
        let mut tokens = header.split_whitespace();
        tokens.next();
        matches!(tokens.next(), Some("SYSTEM" | "PUBLIC"))
    }

    fn has_parameter_entity_reference(syntax: &str) -> bool {
        let bytes = syntax.as_bytes();
        let mut quote = None;
        let mut index = 0;
        while index < bytes.len() {
            if quote.is_none() && bytes[index..].starts_with(b"<!--") {
                index = bytes[index + 4..]
                    .windows(3)
                    .position(|window| window == b"-->")
                    .map_or(bytes.len(), |end| index + 4 + end + 3);
                continue;
            }
            if quote.is_none() && bytes[index..].starts_with(b"<?") {
                index = bytes[index + 2..]
                    .windows(2)
                    .position(|window| window == b"?>")
                    .map_or(bytes.len(), |end| index + 2 + end + 2);
                continue;
            }
            let byte = bytes[index];
            if let Some(delimiter) = quote {
                if byte == delimiter {
                    quote = None;
                }
            } else if matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if byte == b'%' {
                let mut end = index + 1;
                while bytes.get(end).is_some_and(|byte| {
                    byte.is_ascii_alphanumeric()
                        || !byte.is_ascii()
                        || matches!(byte, b'_' | b'-' | b'.' | b':')
                }) {
                    end += 1;
                }
                if end > index + 1 && bytes.get(end) == Some(&b';') {
                    return true;
                }
            }
            index += 1;
        }
        false
    }

    #[derive(Debug)]
    struct ContentModel {
        kind: i32,
        quantifier: i32,
        name: Option<String>,
        children: Vec<Self>,
    }

    fn element_content_model(source: &str) -> Option<ContentModel> {
        struct Parser<'a> {
            source: &'a [u8],
            index: usize,
        }
        impl Parser<'_> {
            fn whitespace(&mut self) {
                while self
                    .source
                    .get(self.index)
                    .is_some_and(u8::is_ascii_whitespace)
                {
                    self.index += 1;
                }
            }

            fn model(&mut self, depth: usize) -> Option<ContentModel> {
                if depth > 128 {
                    return None;
                }
                self.whitespace();
                if self.source.get(self.index) == Some(&b'(') {
                    self.index += 1;
                    self.whitespace();
                    let mixed =
                        self.source.get(self.index..self.index + 7) == Some(b"#PCDATA" as &[u8]);
                    let mut children = Vec::new();
                    let mut separator = None;
                    loop {
                        self.whitespace();
                        if self.source.get(self.index) == Some(&b')') {
                            self.index += 1;
                            break;
                        }
                        let is_pcdata = mixed
                            && self.source.get(self.index..self.index + 7)
                                == Some(b"#PCDATA" as &[u8]);
                        if is_pcdata {
                            self.index += 7;
                        } else {
                            children.push(self.model(depth + 1)?);
                        }
                        self.whitespace();
                        let delimiter = *self.source.get(self.index)?;
                        if delimiter == b')' {
                            self.index += 1;
                            break;
                        }
                        if !matches!(delimiter, b',' | b'|')
                            || separator.is_some_and(|old| old != delimiter)
                        {
                            return None;
                        }
                        separator = Some(delimiter);
                        self.index += 1;
                    }
                    let quantifier = self.quantifier();
                    Some(ContentModel {
                        kind: if mixed {
                            3
                        } else if separator == Some(b'|') {
                            5
                        } else {
                            6
                        },
                        quantifier,
                        name: None,
                        children,
                    })
                } else {
                    let start = self.index;
                    while self.source.get(self.index).is_some_and(|byte| {
                        !byte.is_ascii_whitespace()
                            && !matches!(byte, b',' | b'|' | b')' | b'?' | b'*' | b'+')
                    }) {
                        self.index += 1;
                    }
                    if start == self.index {
                        return None;
                    }
                    let name = core::str::from_utf8(&self.source[start..self.index])
                        .ok()?
                        .to_owned();
                    Some(ContentModel {
                        kind: 4,
                        quantifier: self.quantifier(),
                        name: Some(name),
                        children: Vec::new(),
                    })
                }
            }

            fn quantifier(&mut self) -> i32 {
                let quantifier = match self.source.get(self.index) {
                    Some(b'?') => 1,
                    Some(b'*') => 2,
                    Some(b'+') => 3,
                    _ => 0,
                };
                if quantifier != 0 {
                    self.index += 1;
                }
                quantifier
            }
        }
        let mut parser = Parser {
            source: source.as_bytes(),
            index: 0,
        };
        let model = parser.model(0)?;
        parser.whitespace();
        (parser.index == parser.source.len()).then_some(model)
    }

    fn content_model_exceeds_depth(source: &str) -> bool {
        let mut depth = 0usize;
        for byte in source.bytes() {
            match byte {
                b'(' => {
                    depth += 1;
                    if depth > 128 {
                        return true;
                    }
                }
                b')' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        false
    }

    fn content_model_object(model: ContentModel, vm: &VirtualMachine) -> PyObjectRef {
        let children = model
            .children
            .into_iter()
            .map(|child| content_model_object(child, vm))
            .collect();
        vm.ctx
            .new_tuple(vec![
                vm.ctx.new_int(model.kind).into(),
                vm.ctx.new_int(model.quantifier).into(),
                model
                    .name
                    .map_or_else(|| vm.ctx.none(), |name| vm.ctx.new_str(name).into()),
                vm.ctx.new_tuple(children).into(),
            ])
            .into()
    }

    impl Doctype {
        fn from_reader<R: Read>(
            parser: &xml::EventReader<R>,
            event: &XmlEvent,
            has_parameter_entity_reference: bool,
            skipped_parameter_entities: Vec<String>,
        ) -> Option<Self> {
            let XmlEvent::Doctype { syntax } = event else {
                return None;
            };
            let ids = parser.doctype_ids()?;
            let mut quote = None;
            let has_internal_subset = syntax.chars().any(|character| {
                if let Some(delimiter) = quote {
                    if character == delimiter {
                        quote = None;
                    }
                } else if matches!(character, '\'' | '"') {
                    quote = Some(character);
                } else if character == '[' {
                    return true;
                }
                false
            });
            Some(Self {
                syntax: syntax.clone(),
                name: ids.name().to_owned(),
                system_id: ids.system_id().map(str::to_owned),
                public_id: ids.public_id().map(str::to_owned),
                has_internal_subset,
                has_parameter_entity_reference,
                skipped_parameter_entities,
            })
        }
    }

    fn is_incomplete(error: &xml::reader::Error) -> bool {
        match error.kind() {
            xml::reader::ErrorKind::UnexpectedEof => true,
            xml::reader::ErrorKind::Syntax(message) => {
                message.starts_with("Unexpected end of stream")
                    || message.starts_with("Unclosed <![CDATA[")
            }
            _ => false,
        }
    }

    /// Tracks byte offsets of line starts as bytes are handed to the parser
    /// thread, so that `TextPosition` (row/column, in characters) can be
    /// translated into an approximation of libexpat's `CurrentByteIndex`
    /// without retaining the whole document. Only the as-yet-unscanned tail
    /// of the line currently being read is kept (bounded by how far
    /// `BufReader` has read ahead of the last queried column, not by
    /// document or even line length: already-converted bytes are dropped
    /// from the front as soon as a query passes them, and `line_scanned_*`
    /// remembers how far conversion has gotten so each query only rescans
    /// new characters instead of the whole line from its start -- essential
    /// for documents with very long or even single-line content, otherwise
    /// converting column N would cost O(N) *every* time and the total cost
    /// across a whole line would be quadratic again). Older, already-passed
    /// lines are represented by their starting offset only, so a position
    /// that lags behind falls back to treating the column as a byte count.
    /// This is exact for ASCII documents where the queried position is on
    /// the line most recently read (which covers all callers that rely on
    /// it, e.g. genshi) and only approximate otherwise -- the same guarantee
    /// the previous whole-buffer implementation made.
    #[derive(Default)]
    struct LineTracker {
        total_read: usize,
        line_starts: Vec<usize>,
        /// Unscanned tail of the current line (bytes at or after
        /// `line_scanned_bytes`).
        current_line: VecDeque<u8>,
        /// How many bytes/chars of the current line have already been
        /// converted and dropped from the front of `current_line`.
        line_scanned_bytes: usize,
        line_scanned_chars: usize,
    }

    impl LineTracker {
        fn new() -> Self {
            Self {
                total_read: 0,
                line_starts: vec![0],
                current_line: VecDeque::new(),
                line_scanned_bytes: 0,
                line_scanned_chars: 0,
            }
        }

        fn record(&mut self, bytes: &[u8]) {
            // Everything before the last newline in this chunk belongs to
            // lines that are already finished, so only their start offsets
            // are worth keeping; the tail after it is what a later position
            // query can still be asked about.
            let mut rest = bytes;
            while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
                self.total_read += nl + 1;
                self.line_starts.push(self.total_read);
                self.current_line.clear();
                self.line_scanned_bytes = 0;
                self.line_scanned_chars = 0;
                rest = &rest[nl + 1..];
            }
            self.current_line.extend(rest.iter().copied());
            self.total_read += rest.len();
        }

        /// Walk at most `take` UTF-8 characters from the front of `slice`,
        /// returning how many bytes and characters that covered. Stops early
        /// on a byte that cannot start a character or on a character the
        /// slice does not hold in full.
        fn advance_chars(slice: &[u8], take: usize) -> (usize, usize) {
            let mut bytes = 0usize;
            let mut chars = 0usize;
            while chars < take && bytes < slice.len() {
                let lead = slice[bytes];
                let width = if lead < 0x80 {
                    1
                } else if lead >> 5 == 0b110 {
                    2
                } else if lead >> 4 == 0b1110 {
                    3
                } else if lead >> 3 == 0b11110 {
                    4
                } else {
                    break;
                };
                if bytes + width > slice.len() {
                    break;
                }
                bytes += width;
                chars += 1;
            }
            (bytes, chars)
        }

        fn byte_index_for(&mut self, pos: xml::common::TextPosition) -> i64 {
            let row = pos.row() as usize;
            let line_start = self
                .line_starts
                .get(row)
                .copied()
                .unwrap_or(self.total_read);
            if row + 1 != self.line_starts.len() {
                // The line's bytes have already been discarded; approximate
                // assuming one byte per character (see doc comment above).
                return (line_start + pos.column() as usize) as i64;
            }
            // The position is on the line currently being read: extend the
            // exact character-to-byte conversion only up to the new column,
            // reusing what a previous query on this line already converted.
            let target_chars = pos.column() as usize;
            if target_chars > self.line_scanned_chars {
                let take = target_chars - self.line_scanned_chars;
                // Walk only as far as the requested column, over the deque's
                // two halves in place. Decoding (or even merely validating)
                // the whole buffered tail here made every query cost the
                // length of the read-ahead, which on a document that is one
                // long line -- the shape `xml.etree` writes -- turned a
                // per-event position lookup into quadratic work.
                let (front, back) = self.current_line.as_slices();
                let (mut consumed_bytes, mut consumed_chars) = Self::advance_chars(front, take);
                if consumed_chars < take && consumed_bytes == front.len() {
                    let (b, c) = Self::advance_chars(back, take - consumed_chars);
                    consumed_bytes += b;
                    consumed_chars += c;
                }
                self.current_line.drain(..consumed_bytes);
                self.line_scanned_bytes += consumed_bytes;
                self.line_scanned_chars += consumed_chars;
            }
            (line_start + self.line_scanned_bytes) as i64
        }
    }

    /// Shared feed buffer between the calling (Python) thread and the parser
    /// thread. See the module-level doc comment for the full protocol.
    #[cfg(not(target_arch = "wasm32"))]
    struct FeedInner {
        data: VecDeque<u8>,
        /// Set once `Parse(..., isfinal=True)` has been fed: no more bytes
        /// will ever be pushed, so `read()` should report EOF once drained.
        closed: bool,
        /// Set by the calling thread to force the parser thread to stop
        /// (used when the parser object is dropped, or after a handler
        /// raised, so the background thread doesn't linger uselessly -- it
        /// remains parked otherwise, since `stopped` is only ever cleared by
        /// constructing a brand new `FeedBuffer`).
        stopped: bool,
        /// Number of `push` calls so far. Stamped onto every `NeedMore` so
        /// the calling thread can tell a "waiting for the chunk you just
        /// gave me" report from one the parser thread produced *before*
        /// that chunk landed. See `pump`.
        pushes: u64,
    }

    #[cfg(not(target_arch = "wasm32"))]
    struct FeedBuffer {
        inner: std::sync::Mutex<FeedInner>,
        cv: std::sync::Condvar,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl FeedBuffer {
        fn new() -> Self {
            Self {
                inner: std::sync::Mutex::new(FeedInner {
                    data: VecDeque::new(),
                    closed: false,
                    stopped: false,
                    pushes: 0,
                }),
                cv: std::sync::Condvar::new(),
            }
        }

        /// Hand `chunk` to the parser thread, returning the push count this
        /// chunk was given so `pump` can recognise reports about it.
        fn push(&self, chunk: &[u8], isfinal: bool) -> u64 {
            let mut inner = self.inner.lock().unwrap();
            inner.data.extend(chunk.iter().copied());
            if isfinal {
                inner.closed = true;
            }
            inner.pushes += 1;
            let pushes = inner.pushes;
            self.cv.notify_all();
            pushes
        }

        fn stop(&self) {
            let mut inner = self.inner.lock().unwrap();
            inner.stopped = true;
            self.cv.notify_all();
        }
    }

    /// `std::io::Read` implementation that pulls bytes fed via `FeedBuffer`,
    /// blocking (via condvar) when the buffer is drained and neither closed
    /// nor stopped. Runs entirely on the parser thread.
    #[cfg(not(target_arch = "wasm32"))]
    struct FeedReader {
        buf: alloc::sync::Arc<FeedBuffer>,
        tracker: Rc<RefCell<LineTracker>>,
        events_tx: std::sync::mpsc::Sender<ParserMsg>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl Read for FeedReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let mut inner = self.buf.inner.lock().unwrap();
            loop {
                if inner.stopped {
                    return Ok(0);
                }
                if !inner.data.is_empty() {
                    let n = out.len().min(inner.data.len());
                    for slot in &mut out[..n] {
                        *slot = inner.data.pop_front().unwrap();
                    }
                    self.tracker.borrow_mut().record(&out[..n]);
                    return Ok(n);
                }
                if inner.closed {
                    return Ok(0);
                }
                // The buffer is drained but more data may still arrive:
                // tell the calling thread we're about to block, so it
                // regains control instead of waiting on us indefinitely.
                let _ = self.events_tx.send(ParserMsg::NeedMore(inner.pushes));
                inner = self.buf.cv.wait(inner).unwrap();
            }
        }
    }

    /// One parser event, prepared on the parser thread (position captured
    /// there, since only that thread drives the `EventReader`) and handed to
    /// the calling thread for dispatch.
    #[cfg(not(target_arch = "wasm32"))]
    struct PreparedEvent {
        event: XmlEvent,
        doctype: Option<Box<Doctype>>,
        xml_declaration: Option<XmlDeclarationInfo>,
        line: i64,
        column: i64,
        byte_index: i64,
    }

    #[derive(Clone, Debug)]
    struct XmlDeclarationInfo {
        encoding: Option<String>,
        standalone: Option<bool>,
    }

    fn xml_decl_attribute(declaration: &[u8], name: &[u8]) -> Option<String> {
        let mut cursor = 5;
        while cursor < declaration.len() {
            while declaration.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            let start = cursor;
            while declaration
                .get(cursor)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                cursor += 1;
            }
            if start == cursor {
                cursor += 1;
                continue;
            }
            let attribute = &declaration[start..cursor];
            while declaration.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            if declaration.get(cursor) != Some(&b'=') {
                continue;
            }
            cursor += 1;
            while declaration.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            let quote = *declaration.get(cursor)?;
            if !matches!(quote, b'\'' | b'"') {
                return None;
            }
            cursor += 1;
            let value_start = cursor;
            while declaration.get(cursor).is_some_and(|byte| *byte != quote) {
                cursor += 1;
            }
            let value = core::str::from_utf8(&declaration[value_start..cursor]).ok()?;
            if declaration.get(cursor).is_some() {
                cursor += 1;
            }
            if attribute == name {
                return Some(value.to_owned());
            }
        }
        None
    }

    fn parse_xml_declaration(prefix: &[u8]) -> Option<XmlDeclarationInfo> {
        let utf16_declaration = match prefix.get(..2) {
            Some([0xff, 0xfe]) => Some((true, 2)),
            Some([0xfe, 0xff]) => Some((false, 2)),
            _ if prefix.starts_with(b"<\0?\0") => Some((true, 0)),
            _ if prefix.starts_with(b"\0<\0?") => Some((false, 0)),
            _ => None,
        }
        .and_then(|(little_endian, start)| {
            let mut declaration = Vec::new();
            let (pairs, _) = prefix[start..].as_chunks::<2>();
            for pair in pairs {
                let unit = if little_endian {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                };
                if unit > u8::MAX as u16 {
                    return None;
                }
                declaration.push(unit as u8);
                if declaration.ends_with(b"?>") {
                    break;
                }
            }
            Some(declaration)
        });
        let declaration_start = prefix.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(prefix);
        let declaration_start = utf16_declaration.as_deref().unwrap_or(declaration_start);
        if !declaration_start.starts_with(b"<?xml")
            || !declaration_start
                .get(5)
                .is_some_and(u8::is_ascii_whitespace)
        {
            return None;
        }
        let declaration_end = declaration_start
            .windows(2)
            .position(|window| window == b"?>")?;
        let declaration = &declaration_start[..declaration_end];
        Some(XmlDeclarationInfo {
            encoding: xml_decl_attribute(declaration, b"encoding"),
            standalone: xml_decl_attribute(declaration, b"standalone").map(|value| value == "yes"),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    enum ParserMsg {
        /// The parser thread's `Read` call is blocked waiting for more
        /// input: everything fed so far has been turned into events (or
        /// `NeedMore`/`Event` messages already sent), so the calling thread
        /// can safely return control to Python. Carries the `FeedBuffer`
        /// push count observed when the buffer was found empty.
        NeedMore(u64),
        Event(PreparedEvent),
        /// `EndDocument` was reached; the parser thread has exited.
        Finished,
        /// xml-rs reported a parse error, to be raised on the calling thread.
        Error(xml::reader::Error),
    }

    /// Set, once and permanently, by a `pthread_atfork` child hook the
    /// instant this process is ever the child of a `fork()`. See the "Fork
    /// safety" section of the module doc comment for why both a fresh
    /// thread spawn and touching an inherited `ParserStream` are unsafe
    /// once this is set, and why the fix is the same for both: never
    /// spawn/touch a `Backend::Threaded` thread again for the rest of the
    /// process.
    #[cfg(not(target_arch = "wasm32"))]
    static POST_FORK_CHILD: AtomicBool = AtomicBool::new(false);

    /// libc child-side `pthread_atfork` callback. Must be async-fork-safe:
    /// a single relaxed-ish store is as simple as it gets.
    #[cfg(all(not(target_arch = "wasm32"), unix))]
    extern "C" fn mark_post_fork_child() {
        POST_FORK_CHILD.store(true, Ordering::SeqCst);
    }

    /// Register `mark_post_fork_child` exactly once. Called from
    /// `ParserStream::try_spawn`, i.e. lazily, the first time this process
    /// spawns a parser thread -- so a process that never uses the threaded
    /// backend never touches `pthread_atfork` either.
    #[cfg(all(not(target_arch = "wasm32"), unix))]
    fn ensure_atfork_hook_registered() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| unsafe {
            libc::pthread_atfork(None, None, Some(mark_post_fork_child));
        });
    }

    #[cfg(all(not(target_arch = "wasm32"), not(unix)))]
    fn ensure_atfork_hook_registered() {
        // No `fork()` (hence no `pthread_atfork`) on non-unix targets that
        // still support threads (e.g. Windows); `POST_FORK_CHILD` simply
        // never gets set there, which is correct.
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn post_fork_child() -> bool {
        POST_FORK_CHILD.load(Ordering::SeqCst)
    }

    /// A live parser thread plus the channel it reports through. Dropping
    /// this stops the thread (via `FeedBuffer::stop`, which makes its
    /// blocked or future `read()` calls return EOF) and joins it, so a
    /// parser can never leak a thread, whether or not it was ever fully fed
    /// -- *unless* `post_fork_child()`, in which case see `Drop`.
    #[cfg(not(target_arch = "wasm32"))]
    struct ParserStream {
        // `ManuallyDrop` so `Drop` can leak these (instead of destroying a
        // mutex/condvar/channel that may look held by a thread that no
        // longer exists) when this stream predates a fork. See the module
        // doc comment.
        buf: core::mem::ManuallyDrop<alloc::sync::Arc<FeedBuffer>>,
        rx: core::mem::ManuallyDrop<std::sync::Mutex<std::sync::mpsc::Receiver<ParserMsg>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl core::fmt::Debug for ParserStream {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("ParserStream").finish_non_exhaustive()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl ParserStream {
        /// Try to spawn the background parser thread. Returns `None` (rather
        /// than panicking) if the platform claims thread support but
        /// spawning fails at runtime, so the caller can fall back to
        /// `Backend::Sync` for that parser instead of crashing -- and also
        /// returns `None`, unconditionally, once this process is known to be
        /// the child of a `fork()` (see the module doc comment): spawning a
        /// brand new parser thread there has been observed to reliably
        /// crash the process, so `Backend::new` falls back to `Backend::Sync`
        /// in that case too, exactly as if threads were unavailable.
        fn try_spawn(config: xml::ParserConfig, parameter_entity_parsing: i32) -> Option<Self> {
            ensure_atfork_hook_registered();
            if post_fork_child() {
                return None;
            }
            let buf = alloc::sync::Arc::new(FeedBuffer::new());
            let (tx, rx) = std::sync::mpsc::channel();
            let thread_buf = alloc::sync::Arc::clone(&buf);
            let thread = std::thread::Builder::new()
                .name("pyexpat-parser".into())
                .spawn(move || run_parser_thread(config, thread_buf, tx, parameter_entity_parsing))
                .ok()?;
            Some(Self {
                buf: core::mem::ManuallyDrop::new(buf),
                rx: core::mem::ManuallyDrop::new(std::sync::Mutex::new(rx)),
                thread: Some(thread),
            })
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl Drop for ParserStream {
        fn drop(&mut self) {
            if post_fork_child() {
                // This stream (and the parser thread it names) predates a
                // fork this process has since undergone. `fork()` only ever
                // keeps the calling thread, so `self.thread` no longer
                // refers to a live thread, and `self.buf`/`self.rx`'s
                // internal mutex/condvar may have been held (or, for the
                // condvar, be mid-`wait`, which reacquires the mutex
                // internally) by that vanished thread at the instant of
                // `fork()`. Locking, notifying, joining, or even just
                // *destroying* a POSIX mutex/condvar that looks held is
                // undefined behavior, so touch none of it: forget the
                // thread handle (skips the implicit detach a `JoinHandle`
                // would otherwise perform) and leak the buffer/channel
                // instead of running their destructors. One abandoned
                // parser thread/buffer per fork-with-a-parser-mid-flight is
                // a bounded, one-time leak, not a live hazard.
                if let Some(handle) = self.thread.take() {
                    core::mem::forget(handle);
                }
                return;
            }
            self.buf.stop();
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
            // Safety: not reached on the post-fork leak path above, and
            // `drop` runs at most once, so these are never touched again.
            unsafe {
                core::mem::ManuallyDrop::drop(&mut self.rx);
                core::mem::ManuallyDrop::drop(&mut self.buf);
            }
        }
    }

    /// Body of the parser thread: drives a single long-lived `EventReader`
    /// over the `FeedBuffer`-backed `Read`, sending every event (and
    /// position/EOF/error notifications) back over `tx`. Never touches the
    /// VM or calls into Python -- handlers are dispatched by the calling
    /// thread from the messages sent here, exactly as the module doc
    /// comment requires.
    #[cfg(not(target_arch = "wasm32"))]
    fn run_parser_thread(
        config: xml::ParserConfig,
        buf: alloc::sync::Arc<FeedBuffer>,
        tx: std::sync::mpsc::Sender<ParserMsg>,
        parameter_entity_parsing: i32,
    ) {
        let tracker = Rc::new(RefCell::new(LineTracker::new()));
        let reader = FeedReader {
            buf,
            tracker: Rc::clone(&tracker),
            events_tx: tx.clone(),
        };
        let reader = DoctypeFilterReader::new(reader, parameter_entity_parsing);
        let mut parser = config.create_reader(std::io::BufReader::new(reader));
        loop {
            match parser.next() {
                Ok(XmlEvent::EndDocument) => {
                    let _ = tx.send(ParserMsg::Finished);
                    break;
                }
                Ok(event) => {
                    if let XmlEvent::Doctype { syntax } = &event {
                        let references = parser.source().get_ref().entity_references();
                        let replacements = dtd_entity_replacements(syntax, references);
                        let _ = parser.add_entities(replacements);
                    }
                    let xml_declaration = parser.source().get_ref().xml_declaration();
                    let pos = parser.position();
                    let byte_index = tracker.borrow_mut().byte_index_for(pos);
                    let msg = ParserMsg::Event(PreparedEvent {
                        doctype: Doctype::from_reader(
                            &parser,
                            &event,
                            parser.source().get_ref().has_parameter_entity_reference(),
                            parser.source().get_ref().skipped_parameter_entities(),
                        )
                        .map(Box::new),
                        event,
                        xml_declaration,
                        line: pos.row() as i64 + 1,
                        column: pos.column() as i64,
                        byte_index,
                    });
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.send(ParserMsg::Error(error));
                    break;
                }
            }
        }
    }

    /// `std::io::Read` wrapper that records every byte pulled through it
    /// into a `LineTracker`, so `Backend::Sync` can compute
    /// `CurrentLineNumber`/`CurrentColumnNumber`/`CurrentByteIndex` the same
    /// way `FeedReader` does for `Backend::Threaded` (see that struct and
    /// the module doc comment). Used single-threaded, so a plain
    /// `Rc<RefCell<_>>` (rather than `FeedReader`'s thread-safe handle) is
    /// enough to let the tracker be read back after the reader has been
    /// moved into the `EventReader`.
    struct TrackingReader<R> {
        inner: R,
        tracker: Rc<RefCell<LineTracker>>,
    }

    impl<R: Read> Read for TrackingReader<R> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(out)?;
            self.tracker.borrow_mut().record(&out[..n]);
            Ok(n)
        }
    }

    /// Delays the first XML markup until it can distinguish a document with a
    /// DTD from one without it. xml-rs rejects undeclared parameter-entity
    /// references in an internal subset even when Expat's default parameter
    /// entity mode would leave them unexpanded. Drop only those references
    /// that have no parameter-entity declaration, preserving byte positions.
    struct DoctypeFilterReader<R> {
        inner: R,
        prefix: Vec<u8>,
        ready: VecDeque<u8>,
        content: VecDeque<u8>,
        reference_content: Vec<u8>,
        has_parameter_entity_reference: bool,
        skipped_parameter_entities: Vec<String>,
        xml_declaration: Option<XmlDeclarationInfo>,
        has_bom: bool,
        initialized: bool,
        scan_cursor: usize,
        safe_cursor: usize,
        scan_term: Option<ScanTerm>,
        doctype_scan: Option<DoctypeScan>,
        parameter_entity_parsing: i32,
    }

    #[derive(Clone, Copy)]
    enum ScanTerm {
        Comment,
        ProcessingInstruction,
        Declaration,
    }

    struct DoctypeScan {
        cursor: usize,
        quote: Option<u8>,
        subset_depth: usize,
        in_comment: bool,
        comment_match: usize,
        in_pi: bool,
        pi_question: bool,
    }

    impl<R> DoctypeFilterReader<R> {
        fn new(inner: R, parameter_entity_parsing: i32) -> Self {
            Self {
                inner,
                prefix: Vec::new(),
                ready: VecDeque::new(),
                content: VecDeque::new(),
                reference_content: Vec::new(),
                has_parameter_entity_reference: false,
                skipped_parameter_entities: Vec::new(),
                xml_declaration: None,
                has_bom: false,
                initialized: false,
                scan_cursor: 0,
                safe_cursor: 0,
                scan_term: None,
                doctype_scan: None,
                parameter_entity_parsing,
            }
        }

        fn first_markup(&mut self) -> Option<(usize, usize, bool)> {
            let prefix = &self.prefix;
            let utf16_declaration = if b"<\0?\0".starts_with(prefix) {
                Some(b"?\0>\0".as_slice())
            } else if b"\0<\0?".starts_with(prefix) {
                Some(b"\0?\0>".as_slice())
            } else if prefix.starts_with(b"<\0?\0") {
                Some(b"?\0>\0".as_slice())
            } else if prefix.starts_with(b"\0<\0?") {
                Some(b"\0?\0>".as_slice())
            } else {
                None
            };
            if let Some(end_marker) = utf16_declaration
                && (prefix.len() < 4 || !prefix.windows(end_marker.len()).any(|w| w == end_marker))
            {
                return None;
            }
            loop {
                if let Some(term) = self.scan_term {
                    let marker: &[u8] = match term {
                        ScanTerm::Comment => b"-->",
                        ScanTerm::ProcessingInstruction => b"?>",
                        ScanTerm::Declaration => b">",
                    };
                    if let Some(relative) = prefix[self.scan_cursor..]
                        .windows(marker.len())
                        .position(|window| window == marker)
                    {
                        self.scan_cursor += relative + marker.len();
                        self.safe_cursor = self.scan_cursor;
                        self.scan_term = None;
                        continue;
                    }
                    self.scan_cursor = prefix.len().saturating_sub(marker.len() - 1);
                    return None;
                }
                let Some(relative) = prefix[self.scan_cursor..]
                    .iter()
                    .position(|byte| *byte == b'<')
                else {
                    self.scan_cursor = prefix.len();
                    self.safe_cursor = prefix.len();
                    return None;
                };
                let start = self.scan_cursor + relative;
                let rest = &prefix[start..];
                const TOKENS: [&[u8]; 3] = [b"<!--", b"<?", b"<!DOCTYPE"];
                if TOKENS
                    .iter()
                    .any(|token| rest.len() < token.len() && token.starts_with(rest))
                {
                    self.scan_cursor = start;
                    return None;
                }
                if rest.starts_with(b"<!--") {
                    self.scan_cursor = start + b"<!--".len();
                    self.scan_term = Some(ScanTerm::Comment);
                } else if rest.starts_with(b"<?") {
                    self.scan_cursor = start + b"<?".len();
                    self.scan_term = Some(ScanTerm::ProcessingInstruction);
                } else if rest.starts_with(b"<!DOCTYPE") {
                    return Some((start, start + b"<!DOCTYPE".len(), true));
                } else if rest.starts_with(b"<!") {
                    self.scan_cursor = start + 2;
                    self.scan_term = Some(ScanTerm::Declaration);
                } else {
                    return Some((start, start + 1, false));
                }
            }
        }

        fn doctype_end(&mut self, start: usize) -> Option<usize> {
            let prefix = &self.prefix;
            let scan = self.doctype_scan.get_or_insert(DoctypeScan {
                cursor: start,
                quote: None,
                subset_depth: 0,
                in_comment: false,
                comment_match: 0,
                in_pi: false,
                pi_question: false,
            });
            while scan.cursor < prefix.len() {
                if scan.in_pi {
                    let byte = prefix[scan.cursor];
                    scan.cursor += 1;
                    if byte == b'>' && scan.pi_question {
                        scan.in_pi = false;
                    }
                    scan.pi_question = byte == b'?';
                    continue;
                }
                if scan.in_comment {
                    let byte = prefix[scan.cursor];
                    scan.cursor += 1;
                    if byte == b'-' {
                        scan.comment_match = (scan.comment_match + 1).min(2);
                    } else if byte == b'>' && scan.comment_match == 2 {
                        scan.in_comment = false;
                        scan.comment_match = 0;
                    } else {
                        scan.comment_match = 0;
                    }
                    continue;
                }
                if scan.quote.is_none() && prefix[scan.cursor..].starts_with(b"<!--") {
                    scan.in_comment = true;
                    scan.cursor += 4;
                    continue;
                }
                if scan.quote.is_none() && b"<!--".starts_with(&prefix[scan.cursor..]) {
                    break;
                }
                if scan.quote.is_none() && prefix[scan.cursor..].starts_with(b"<?") {
                    scan.in_pi = true;
                    scan.pi_question = false;
                    scan.cursor += 2;
                    continue;
                }
                if scan.quote.is_none() && b"<?".starts_with(&prefix[scan.cursor..]) {
                    break;
                }
                let byte = prefix[scan.cursor];
                if let Some(delimiter) = scan.quote {
                    if byte == delimiter {
                        scan.quote = None;
                    }
                } else if matches!(byte, b'\'' | b'"') {
                    scan.quote = Some(byte);
                } else if byte == b'[' {
                    scan.subset_depth += 1;
                } else if byte == b']' {
                    scan.subset_depth = scan.subset_depth.saturating_sub(1);
                } else if byte == b'>' && scan.subset_depth == 0 {
                    let end = scan.cursor + 1;
                    self.doctype_scan = None;
                    return Some(end);
                }
                scan.cursor += 1;
            }
            None
        }

        fn filter_undefined_parameter_references(
            doctype: &mut [u8],
            parameter_entity_parsing: i32,
            standalone: bool,
            encoding: Option<&str>,
        ) -> Vec<String> {
            let mut declared = std::collections::HashSet::new();
            let mut skipped = Vec::new();
            let mut index = 0;
            let mut quote = None;
            while index < doctype.len() {
                if quote.is_none() && doctype[index..].starts_with(b"<!--") {
                    index = doctype[index + 4..]
                        .windows(3)
                        .position(|window| window == b"-->")
                        .map_or(doctype.len(), |end| index + 4 + end + 3);
                    continue;
                }
                if quote.is_none() && doctype[index..].starts_with(b"<?") {
                    index = doctype[index + 2..]
                        .windows(2)
                        .position(|window| window == b"?>")
                        .map_or(doctype.len(), |end| index + 2 + end + 2);
                    continue;
                }
                let byte = doctype[index];
                if let Some(delimiter) = quote {
                    if byte == delimiter {
                        quote = None;
                    }
                } else if matches!(byte, b'\'' | b'"') {
                    quote = Some(byte);
                } else if doctype[index..].starts_with(b"<!ENTITY") {
                    let declaration_start = index;
                    index += b"<!ENTITY".len();
                    while doctype.get(index).is_some_and(u8::is_ascii_whitespace) {
                        index += 1;
                    }
                    let is_parameter = doctype.get(index) == Some(&b'%');
                    if is_parameter {
                        index += 1;
                        while doctype.get(index).is_some_and(u8::is_ascii_whitespace) {
                            index += 1;
                        }
                        let name_start = index;
                        while doctype.get(index).is_some_and(|byte| {
                            byte.is_ascii_alphanumeric()
                                || !byte.is_ascii()
                                || matches!(byte, b'_' | b'-' | b'.' | b':')
                        }) {
                            index += 1;
                        }
                        if name_start < index {
                            declared.insert(doctype[name_start..index].to_vec());
                        }
                    }
                    let mut declaration_quote = None;
                    while index < doctype.len() {
                        let current = doctype[index];
                        if let Some(delimiter) = declaration_quote {
                            if current == delimiter {
                                declaration_quote = None;
                            }
                        } else if matches!(current, b'\'' | b'"') {
                            declaration_quote = Some(current);
                        } else if current == b'>' {
                            index += 1;
                            break;
                        }
                        index += 1;
                    }
                    if index == declaration_start {
                        index += 1;
                    }
                    continue;
                } else if byte == b'%' {
                    let start = index;
                    let mut end = index + 1;
                    while doctype.get(end).is_some_and(|byte| {
                        byte.is_ascii_alphanumeric()
                            || !byte.is_ascii()
                            || matches!(byte, b'_' | b'-' | b'.' | b':')
                    }) {
                        end += 1;
                    }
                    if end > start + 1 && doctype.get(end) == Some(&b';') {
                        let name = &doctype[start + 1..end];
                        let parse_entities = parameter_entity_parsing
                            == XML_PARAM_ENTITY_PARSING_ALWAYS
                            || (parameter_entity_parsing
                                == XML_PARAM_ENTITY_PARSING_UNLESS_STANDALONE
                                && !standalone);
                        let preserve_for_standalone_error = parameter_entity_parsing
                            == XML_PARAM_ENTITY_PARSING_ALWAYS
                            && standalone;
                        if parse_entities && !declared.contains(name) {
                            skipped.push(decode_scanned_xml_name(name, encoding));
                        }
                        if !(declared.contains(name) && parse_entities)
                            && !preserve_for_standalone_error
                        {
                            for byte in &mut doctype[start..=end] {
                                if *byte != b'\n' && *byte != b'\r' {
                                    *byte = b' ';
                                }
                            }
                        }
                        index = end;
                    }
                }
                index += 1;
            }
            skipped
        }

        fn initialize(&mut self) -> std::io::Result<()>
        where
            R: Read,
        {
            if self.initialized {
                return Ok(());
            }
            loop {
                let partial_utf8_bom = matches!(self.prefix.as_slice(), [0xef] | [0xef, 0xbb]);
                if let Some((start, end, is_doctype)) = self.first_markup() {
                    if self.safe_cursor != 0
                        && self.safe_cursor <= start
                        && !(self.has_bom
                            && (self.prefix.starts_with(&[0xff, 0xfe])
                                || self.prefix.starts_with(&[0xfe, 0xff])))
                        && !partial_utf8_bom
                    {
                        if self.xml_declaration.is_none() {
                            self.xml_declaration = parse_xml_declaration(&self.prefix);
                        }
                        self.ready.extend(self.prefix.drain(..self.safe_cursor));
                        self.scan_cursor = start - self.safe_cursor;
                        self.safe_cursor = 0;
                        return Ok(());
                    }
                    if is_doctype {
                        if let Some(doctype_end) = self.doctype_end(end) {
                            self.initialized = true;
                            self.has_parameter_entity_reference =
                                core::str::from_utf8(&self.prefix[start..doctype_end])
                                    .is_ok_and(has_parameter_entity_reference);
                            let declaration_start = self
                                .prefix
                                .strip_prefix(&[0xef, 0xbb, 0xbf])
                                .unwrap_or(&self.prefix);
                            if self.xml_declaration.is_none() {
                                self.xml_declaration = parse_xml_declaration(&self.prefix);
                            }
                            let declaration_start =
                                declaration_start.strip_prefix(&b"<?xml"[..]).and_then(|_| {
                                    let end = declaration_start
                                        .windows(2)
                                        .position(|window| window == b"?>")?;
                                    Some(&declaration_start[..end + 2])
                                });
                            let standalone = self.xml_declaration.as_ref().map_or_else(
                                || {
                                    declaration_start
                                        .and_then(|declaration| {
                                            xml_decl_attribute(declaration, b"standalone")
                                        })
                                        .is_some_and(|value| value == "yes")
                                },
                                |declaration| declaration.standalone == Some(true),
                            );
                            self.skipped_parameter_entities =
                                Self::filter_undefined_parameter_references(
                                    &mut self.prefix[start..doctype_end],
                                    self.parameter_entity_parsing,
                                    standalone,
                                    self.xml_declaration
                                        .as_ref()
                                        .and_then(|declaration| declaration.encoding.as_deref()),
                                );
                            let tail = self.prefix.split_off(doctype_end);
                            self.reference_content.extend_from_slice(&tail);
                            self.content.extend(tail);
                            break;
                        }
                    } else {
                        self.initialized = true;
                        let _ = end;
                        break;
                    }
                }
                if self.scan_term.is_none()
                    && self.safe_cursor != 0
                    && !(self.has_bom
                        && (self.prefix.starts_with(&[0xff, 0xfe])
                            || self.prefix.starts_with(&[0xfe, 0xff])))
                    && !partial_utf8_bom
                {
                    if self.xml_declaration.is_none() {
                        self.xml_declaration = parse_xml_declaration(&self.prefix);
                    }
                    self.ready.extend(self.prefix.drain(..self.safe_cursor));
                    self.scan_cursor = 0;
                    self.safe_cursor = 0;
                    return Ok(());
                }
                let mut input = [0; 4096];
                let count = self.inner.read(&mut input)?;
                if count == 0 {
                    self.initialized = true;
                    break;
                }
                let waiting_after_bom = self.has_bom
                    && self.prefix.len() == 2
                    && (self.prefix.starts_with(&[0xff, 0xfe])
                        || self.prefix.starts_with(&[0xfe, 0xff]));
                self.prefix.extend_from_slice(&input[..count]);
                self.has_bom |= self.prefix.starts_with(&[0xff, 0xfe])
                    || self.prefix.starts_with(&[0xfe, 0xff]);
                if waiting_after_bom {
                    self.scan_cursor = 0;
                    self.safe_cursor = 0;
                }
            }
            if self.xml_declaration.is_none() {
                self.xml_declaration = parse_xml_declaration(&self.prefix);
            }
            if !self.has_bom && self.prefix.starts_with(b"<\0?\0") {
                self.ready.extend([0xff, 0xfe]);
            } else if !self.has_bom && self.prefix.starts_with(b"\0<\0?") {
                self.ready.extend([0xfe, 0xff]);
            }
            self.ready.extend(self.prefix.drain(..));
            Ok(())
        }

        fn xml_declaration(&self) -> Option<XmlDeclarationInfo> {
            self.xml_declaration.clone()
        }

        fn has_parameter_entity_reference(&self) -> bool {
            self.has_parameter_entity_reference
        }

        fn skipped_parameter_entities(&self) -> Vec<String> {
            self.skipped_parameter_entities.clone()
        }

        fn entity_references(&self) -> Vec<String> {
            let content = &self.reference_content;
            let encoding = self
                .xml_declaration
                .as_ref()
                .and_then(|declaration| declaration.encoding.as_deref());
            let mut names = Vec::new();
            let mut index = 0;
            while index < content.len() {
                if content[index..].starts_with(b"<!--") {
                    index = content[index + 4..]
                        .windows(3)
                        .position(|window| window == b"-->")
                        .map_or(content.len(), |end| index + 4 + end + 3);
                } else if content[index..].starts_with(b"<![CDATA[") {
                    index = content[index + 9..]
                        .windows(3)
                        .position(|window| window == b"]]>")
                        .map_or(content.len(), |end| index + 9 + end + 3);
                } else if content[index..].starts_with(b"<?") {
                    index = content[index + 2..]
                        .windows(2)
                        .position(|window| window == b"?>")
                        .map_or(content.len(), |end| index + 2 + end + 2);
                } else if content[index] == b'<' {
                    let mut quote = None;
                    index += 1;
                    while index < content.len() {
                        let byte = content[index];
                        if let Some(delimiter) = quote {
                            if byte == delimiter {
                                quote = None;
                            } else if byte == b'&' {
                                let start = index + 1;
                                if let Some(relative) =
                                    content[start..].iter().position(|byte| *byte == b';')
                                {
                                    let end = start + relative;
                                    if end > start
                                        && content[start..end].iter().all(|byte| {
                                            byte.is_ascii_alphanumeric()
                                                || !byte.is_ascii()
                                                || matches!(byte, b'_' | b'-' | b'.' | b':')
                                        })
                                    {
                                        names.push(decode_scanned_xml_name(
                                            &content[start..end],
                                            encoding,
                                        ));
                                    }
                                    index = end + 1;
                                    continue;
                                }
                            }
                        } else if matches!(byte, b'\'' | b'"') {
                            quote = Some(byte);
                        } else if byte == b'>' {
                            index += 1;
                            break;
                        }
                        index += 1;
                    }
                } else if content[index] == b'&' {
                    let start = index + 1;
                    if let Some(relative) = content[start..].iter().position(|byte| *byte == b';') {
                        let end = start + relative;
                        if end > start
                            && content[start..end].iter().all(|byte| {
                                byte.is_ascii_alphanumeric()
                                    || !byte.is_ascii()
                                    || matches!(byte, b'_' | b'-' | b'.' | b':')
                            })
                        {
                            names.push(decode_scanned_xml_name(&content[start..end], encoding));
                        }
                        index = end + 1;
                    } else {
                        break;
                    }
                } else {
                    index += 1;
                }
            }
            names
        }
    }

    impl<R: Read> Read for DoctypeFilterReader<R> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.initialize()?;
            let n = out.len().min(self.ready.len());
            for slot in &mut out[..n] {
                *slot = self.ready.pop_front().unwrap();
            }
            if n != 0 {
                return Ok(n);
            }
            let n = out.len().min(self.content.len());
            for slot in &mut out[..n] {
                *slot = self.content.pop_front().unwrap();
            }
            if n != 0 {
                Ok(n)
            } else {
                let n = self.inner.read(out)?;
                Ok(n)
            }
        }
    }

    /// Single-threaded fallback backend, used on targets that cannot spawn
    /// an OS thread (e.g. `wasm32`) or if doing so failed at runtime. See
    /// the module doc comment for the accumulate-and-reparse strategy and
    /// its O(total bytes fed so far) per-`Parse()` cost.
    #[derive(Debug)]
    struct SyncParser {
        config: xml::ParserConfig,
        buffer: Vec<u8>,
        parameter_entity_parsing: i32,
        /// Number of events (including the trailing `EndDocument`, once
        /// reached) already dispatched by a previous call, so a re-parse of
        /// the grown buffer only dispatches events at or beyond this index.
        dispatched_events: usize,
    }

    impl SyncParser {
        fn new(config: xml::ParserConfig, parameter_entity_parsing: i32) -> Self {
            Self {
                config,
                buffer: Vec::new(),
                parameter_entity_parsing,
                dispatched_events: 0,
            }
        }
    }

    /// The two ways a parser can drive `xml-rs`: a background thread when
    /// one can be spawned (`Threaded`), or an in-place accumulate-and-reparse
    /// fallback when it can't (`Sync`). See the module doc comment.
    #[derive(Debug)]
    enum Backend {
        #[cfg(not(target_arch = "wasm32"))]
        Threaded(ParserStream),
        Sync(SyncParser),
    }

    impl Backend {
        /// Prefer a background thread wherever the target supports one;
        /// fall back to the synchronous backend on targets that can't spawn
        /// threads (`wasm32`) or if spawning fails at runtime, so `Parse()`
        /// never panics for lack of threads.
        fn new(config: xml::ParserConfig, parameter_entity_parsing: i32) -> Self {
            #[cfg(not(target_arch = "wasm32"))]
            {
                if let Some(stream) =
                    ParserStream::try_spawn(config.clone(), parameter_entity_parsing)
                {
                    return Self::Threaded(stream);
                }
            }
            Self::Sync(SyncParser::new(config, parameter_entity_parsing))
        }
    }

    #[pyclass]
    impl PyExpatLikeXmlParser {
        fn new(
            namespace_separator: Option<String>,
            intern: OptionalOption<PyObjectRef>,
            parent: Option<PyExpatLikeXmlParserRef>,
            external_entity: Option<ExternalEntity>,
            vm: &VirtualMachine,
        ) -> PyExpatLikeXmlParserRef {
            let intern_dict = match intern {
                OptionalArg::Missing => vm.ctx.new_dict().into(),
                OptionalArg::Present(Some(obj)) => obj,
                OptionalArg::Present(None) => vm.ctx.none(),
            };
            Self {
                namespace_separator,
                base: PyRwLock::new(None),
                current_line: PyRwLock::new(1),
                current_column: PyRwLock::new(0),
                current_byte_index: PyRwLock::new(-1),
                backend: PyRwLock::new(None),
                busy: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                completed: AtomicBool::new(false),
                namespace_stack: PyRwLock::new(Vec::new()),
                external_entities: PyRwLock::new(alloc::collections::BTreeMap::new()),
                reparse_deferral_enabled: AtomicBool::new(true),
                parameter_entity_parsing: AtomicI32::new(XML_PARAM_ENTITY_PARSING_NEVER),
                use_foreign_dtd: AtomicBool::new(false),
                standalone: AtomicBool::new(false),
                external_subset_handled: AtomicBool::new(false),
                started: AtomicBool::new(false),
                external_entity,
                parent,
                start_element: MutableObject::new(vm.ctx.none()),
                end_element: MutableObject::new(vm.ctx.none()),
                character_data: MutableObject::new(vm.ctx.none()),
                entity_decl: MutableObject::new(vm.ctx.none()),
                buffer_text: MutableObject::new(vm.ctx.new_bool(false).into()),
                namespace_prefixes: MutableObject::new(vm.ctx.new_bool(false).into()),
                ordered_attributes: MutableObject::new(vm.ctx.new_bool(false).into()),
                specified_attributes: MutableObject::new(vm.ctx.new_bool(false).into()),
                intern: intern_dict,
                // Additional handlers (stubs for compatibility)
                processing_instruction: MutableObject::new(vm.ctx.none()),
                unparsed_entity_decl: MutableObject::new(vm.ctx.none()),
                notation_decl: MutableObject::new(vm.ctx.none()),
                start_namespace_decl: MutableObject::new(vm.ctx.none()),
                end_namespace_decl: MutableObject::new(vm.ctx.none()),
                comment: MutableObject::new(vm.ctx.none()),
                start_cdata_section: MutableObject::new(vm.ctx.none()),
                end_cdata_section: MutableObject::new(vm.ctx.none()),
                default: MutableObject::new(vm.ctx.none()),
                default_expand: MutableObject::new(vm.ctx.none()),
                not_standalone: MutableObject::new(vm.ctx.none()),
                external_entity_ref: MutableObject::new(vm.ctx.none()),
                start_doctype_decl: MutableObject::new(vm.ctx.none()),
                end_doctype_decl: MutableObject::new(vm.ctx.none()),
                xml_decl: MutableObject::new(vm.ctx.none()),
                element_decl: MutableObject::new(vm.ctx.none()),
                attlist_decl: MutableObject::new(vm.ctx.none()),
                skipped_entity: MutableObject::new(vm.ctx.none()),
            }
            .into_ref(&vm.ctx)
        }

        #[extend_class]
        fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
            let attributes = &class.attributes;

            create_property!(ctx, attributes, "StartElementHandler", class, start_element);
            create_property!(ctx, attributes, "EndElementHandler", class, end_element);
            create_property!(
                ctx,
                attributes,
                "CharacterDataHandler",
                class,
                character_data
            );
            create_property!(ctx, attributes, "EntityDeclHandler", class, entity_decl);
            create_bool_property!(ctx, attributes, "buffer_text", class, buffer_text);
            create_bool_property!(
                ctx,
                attributes,
                "namespace_prefixes",
                class,
                namespace_prefixes
            );
            create_bool_property!(
                ctx,
                attributes,
                "ordered_attributes",
                class,
                ordered_attributes
            );
            create_bool_property!(
                ctx,
                attributes,
                "specified_attributes",
                class,
                specified_attributes
            );
            create_readonly_int_property!(
                ctx,
                attributes,
                "CurrentLineNumber",
                class,
                current_line
            );
            create_readonly_int_property!(
                ctx,
                attributes,
                "CurrentColumnNumber",
                class,
                current_column
            );
            create_readonly_int_property!(
                ctx,
                attributes,
                "CurrentByteIndex",
                class,
                current_byte_index
            );
            // Additional handlers (stubs for compatibility)
            create_property!(
                ctx,
                attributes,
                "ProcessingInstructionHandler",
                class,
                processing_instruction
            );
            create_property!(
                ctx,
                attributes,
                "UnparsedEntityDeclHandler",
                class,
                unparsed_entity_decl
            );
            create_property!(ctx, attributes, "NotationDeclHandler", class, notation_decl);
            create_property!(
                ctx,
                attributes,
                "StartNamespaceDeclHandler",
                class,
                start_namespace_decl
            );
            create_property!(
                ctx,
                attributes,
                "EndNamespaceDeclHandler",
                class,
                end_namespace_decl
            );
            create_property!(ctx, attributes, "CommentHandler", class, comment);
            create_property!(
                ctx,
                attributes,
                "StartCdataSectionHandler",
                class,
                start_cdata_section
            );
            create_property!(
                ctx,
                attributes,
                "EndCdataSectionHandler",
                class,
                end_cdata_section
            );
            create_property!(ctx, attributes, "DefaultHandler", class, default);
            create_property!(
                ctx,
                attributes,
                "DefaultHandlerExpand",
                class,
                default_expand
            );
            create_property!(
                ctx,
                attributes,
                "NotStandaloneHandler",
                class,
                not_standalone
            );
            create_property!(
                ctx,
                attributes,
                "ExternalEntityRefHandler",
                class,
                external_entity_ref
            );
            create_property!(
                ctx,
                attributes,
                "StartDoctypeDeclHandler",
                class,
                start_doctype_decl
            );
            create_property!(
                ctx,
                attributes,
                "EndDoctypeDeclHandler",
                class,
                end_doctype_decl
            );
            create_property!(ctx, attributes, "XmlDeclHandler", class, xml_decl);
            create_property!(ctx, attributes, "ElementDeclHandler", class, element_decl);
            create_property!(ctx, attributes, "AttlistDeclHandler", class, attlist_decl);
            create_property!(
                ctx,
                attributes,
                "SkippedEntityHandler",
                class,
                skipped_entity
            );
        }

        fn create_config(&self) -> xml::ParserConfig {
            xml::ParserConfig::new()
                .cdata_to_characters(false)
                .coalesce_characters(false)
                .ignore_comments(false)
                .whitespace_to_characters(true)
        }

        #[pymethod(name = "GetReparseDeferralEnabled")]
        fn get_reparse_deferral_enabled(zelf: &Py<Self>) -> bool {
            zelf.reparse_deferral_enabled.load(Ordering::Relaxed)
        }

        #[pymethod(name = "SetReparseDeferralEnabled")]
        fn set_reparse_deferral_enabled(zelf: &Py<Self>, enabled: bool) {
            // The streaming backend resumes a suspended parser rather than
            // reparsing an unfinished token, so it needs no deferral to avoid
            // quadratic work. Preserve the setting used by callers that
            // temporarily disable it to flush already supplied input.
            zelf.reparse_deferral_enabled
                .store(enabled, Ordering::Relaxed);
        }

        #[pymethod(name = "SetParamEntityParsing")]
        fn set_param_entity_parsing(zelf: &Py<Self>, flag: i32) -> i32 {
            if zelf.started.load(Ordering::Relaxed) && !zelf.finished.load(Ordering::Relaxed) {
                return 0;
            }
            zelf.parameter_entity_parsing.store(flag, Ordering::Relaxed);
            1
        }

        #[pymethod(name = "UseForeignDTD")]
        fn use_foreign_dtd(
            zelf: &Py<Self>,
            flag: OptionalArg<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if zelf.started.load(Ordering::Relaxed) && !zelf.finished.load(Ordering::Relaxed) {
                return Err(zelf.handler_error(
                    vm,
                    26,
                    "cannot change setting once parsing has begun",
                )?);
            }
            zelf.use_foreign_dtd
                .store(flag.unwrap_or(true), Ordering::Relaxed);
            Ok(())
        }

        #[pymethod(name = "ExternalEntityParserCreate")]
        fn external_entity_parser_create(
            zelf: PyRef<Self>,
            context: Option<PyUtf8StrRef>,
            encoding: OptionalArg<PyUtf8StrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyExpatLikeXmlParserRef> {
            let context = context.map(|s| s.as_str().to_owned());
            let encoding = encoding.into_option().map(|s| s.as_str().to_owned());
            if context.as_ref().is_some_and(|s| s.contains('\0'))
                || encoding.as_ref().is_some_and(|s| s.contains('\0'))
            {
                return Err(vm.new_value_error("embedded null character"));
            }
            let child = Self::new(
                zelf.namespace_separator.clone(),
                OptionalArg::Present(Some(zelf.intern.clone())),
                Some(zelf.clone()),
                Some(ExternalEntity { context, encoding }),
                vm,
            );
            child.reparse_deferral_enabled.store(
                zelf.reparse_deferral_enabled.load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            child.parameter_entity_parsing.store(
                zelf.parameter_entity_parsing.load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            for (source, target) in [
                (&zelf.buffer_text, &child.buffer_text),
                (&zelf.namespace_prefixes, &child.namespace_prefixes),
                (&zelf.ordered_attributes, &child.ordered_attributes),
                (&zelf.specified_attributes, &child.specified_attributes),
                (&zelf.start_element, &child.start_element),
                (&zelf.end_element, &child.end_element),
                (&zelf.character_data, &child.character_data),
                (&zelf.entity_decl, &child.entity_decl),
                (&zelf.processing_instruction, &child.processing_instruction),
                (&zelf.unparsed_entity_decl, &child.unparsed_entity_decl),
                (&zelf.notation_decl, &child.notation_decl),
                (&zelf.start_namespace_decl, &child.start_namespace_decl),
                (&zelf.end_namespace_decl, &child.end_namespace_decl),
                (&zelf.comment, &child.comment),
                (&zelf.start_cdata_section, &child.start_cdata_section),
                (&zelf.end_cdata_section, &child.end_cdata_section),
                (&zelf.default, &child.default),
                (&zelf.default_expand, &child.default_expand),
                (&zelf.not_standalone, &child.not_standalone),
                (&zelf.external_entity_ref, &child.external_entity_ref),
                (&zelf.start_doctype_decl, &child.start_doctype_decl),
                (&zelf.end_doctype_decl, &child.end_doctype_decl),
                (&zelf.xml_decl, &child.xml_decl),
                (&zelf.element_decl, &child.element_decl),
                (&zelf.attlist_decl, &child.attlist_decl),
                (&zelf.skipped_entity, &child.skipped_entity),
            ] {
                *target.write() = source.read().clone();
            }
            Ok(child)
        }

        #[pymethod(name = "SetBase")]
        fn set_base(zelf: &Py<Self>, base: PyStrRef) {
            // Store-only compatibility state for xml.sax locator APIs. The
            // xml-rs backend still does not perform Expat-style base URI
            // resolution for external entities.
            *zelf.base.write() = Some(AsRef::<str>::as_ref(&base).to_owned());
        }

        #[pymethod(name = "GetBase")]
        fn get_base(zelf: &Py<Self>, vm: &VirtualMachine) -> PyObjectRef {
            zelf.base.read().as_ref().map_or_else(
                || vm.ctx.none(),
                |base| vm.ctx.new_str(base.as_str()).into(),
            )
        }

        /// Construct element name with namespace if separator is set
        fn make_name(&self, name: &xml::name::OwnedName, vm: &VirtualMachine) -> String {
            match (&self.namespace_separator, &name.namespace) {
                (Some(sep), Some(ns)) if !ns.is_empty() => {
                    let mut expanded = format!("{}{}{}", ns, sep, name.local_name);
                    if self.namespace_prefixes.read().is(&vm.ctx.true_value)
                        && let Some(prefix) = &name.prefix
                    {
                        expanded.push_str(sep);
                        expanded.push_str(prefix);
                    }
                    expanded
                }
                (None, _) => match &name.prefix {
                    Some(prefix) => format!("{prefix}:{}", name.local_name),
                    None => name.local_name.clone(),
                },
                _ => name.local_name.clone(),
            }
        }

        /// Build a `PyStr` for an element or attribute *name*, memoizing it in
        /// `self.intern` (a plain dict, mirroring libexpat's `intern`
        /// behavior: `PyDict_SetDefault(self->intern, name, name)`) so that a
        /// name repeated across many events -- e.g. the same tag or attribute
        /// key appearing on hundreds of sibling elements -- reuses the same
        /// `PyStr` object instead of allocating and re-hashing a fresh one
        /// each time. Falls back to a plain, non-memoized `PyStr` if
        /// `self.intern` has been replaced with something other than a dict.
        /// Attribute *values* are intentionally left out of this cache, same
        /// as libexpat: they vary far more than names and rarely repeat.
        fn intern_name(&self, vm: &VirtualMachine, name: String) -> PyStrRef {
            let intern_obj = self.intern.clone();
            if let Ok(dict) = intern_obj.downcast::<crate::vm::builtins::PyDict>() {
                if let Ok(Some(existing)) = dict.get_item_opt(name.as_str(), vm)
                    && let Ok(existing) = existing.downcast::<PyStr>()
                {
                    return existing;
                }
                let interned = vm.ctx.new_str(name);
                let _ = dict.set_item(AsRef::<str>::as_ref(&interned), interned.clone().into(), vm);
                interned
            } else {
                PyStr::from(name).into_ref(&vm.ctx)
            }
        }

        fn dispatch_character_data(&self, vm: &VirtualMachine, text: String) -> PyResult<()> {
            let mut rest = text.as_str();
            while let Some(start) = rest.find('\0') {
                if start != 0 {
                    let content = PyStr::from(rest[..start].to_owned()).into_ref(&vm.ctx);
                    invoke_handler(vm, &self.character_data, (content,))?;
                }
                let marker_start = &rest[start + '\0'.len_utf8()..];
                let Some(end) = marker_start.find('\0') else {
                    let content = PyStr::from(rest.to_owned()).into_ref(&vm.ctx);
                    return invoke_handler(vm, &self.character_data, (content,));
                };
                let marker = &marker_start[..end];
                let mut chars = marker.chars();
                let kind = chars.next();
                let name: String = chars.collect();
                let recognized = match kind {
                    Some('E') => self.external_entities.read().contains_key(&name),
                    Some('S') => {
                        self.external_subset_handled.load(Ordering::Relaxed) && !name.is_empty()
                    }
                    _ => false,
                };
                if !recognized {
                    let content = PyStr::from("\u{fdd0}").into_ref(&vm.ctx);
                    invoke_handler(vm, &self.character_data, (content,))?;
                    rest = &rest[start + '\0'.len_utf8()..];
                    continue;
                }
                if kind == Some('S') && self.standalone.load(Ordering::Relaxed) {
                    return Err(self.handler_error(vm, 11, "undefined entity")?);
                }
                match kind {
                    Some('E') => {
                        let ids = self.external_entities.read().get(&name).cloned();
                        if let Some((system_id, public_id)) = ids {
                            let base = self
                                .base
                                .read()
                                .as_ref()
                                .map(|base| vm.ctx.new_str(base.clone()));
                            if invoke_int_handler(
                                vm,
                                &self.external_entity_ref,
                                "ExternalEntityRef",
                                (
                                    vm.ctx.new_str(self.external_entity_context(&name)),
                                    base,
                                    system_id.map(|value| vm.ctx.new_str(value)),
                                    public_id.map(|value| vm.ctx.new_str(value)),
                                ),
                            )? == Some(0)
                            {
                                return Err(self.handler_error(
                                    vm,
                                    21,
                                    "error in processing external entity reference",
                                )?);
                            }
                        }
                    }
                    Some('S') if !vm.is_none(&self.skipped_entity.read()) => {
                        invoke_handler(vm, &self.skipped_entity, (vm.ctx.new_str(name), 0))?;
                    }
                    Some('S') => {}
                    _ => {}
                }
                rest = &marker_start[end + '\0'.len_utf8()..];
            }
            if !rest.is_empty() {
                let content = PyStr::from(rest.to_owned()).into_ref(&vm.ctx);
                invoke_handler(vm, &self.character_data, (content,))?;
            }
            Ok(())
        }

        fn external_entity_context(&self, name: &str) -> String {
            let mut context = name.to_owned();
            if self.namespace_separator.is_some()
                && let Some((namespace, _)) = self.namespace_stack.read().last()
            {
                for (prefix, uri) in namespace.iter() {
                    if (prefix == "xml" && uri == xml::namespace::NS_XML_URI)
                        || (prefix == "xmlns" && uri == xml::namespace::NS_XMLNS_URI)
                    {
                        continue;
                    }
                    context.push('\x0c');
                    context.push_str(prefix);
                    context.push('=');
                    context.push_str(uri);
                }
            }
            context
        }

        /// Dispatch a single parser event to the registered handlers. Runs on
        /// the calling thread only -- see the module doc comment.
        fn dispatch(
            &self,
            vm: &VirtualMachine,
            event: XmlEvent,
            doctype: Option<Box<Doctype>>,
            xml_declaration: Option<XmlDeclarationInfo>,
        ) -> PyResult<()> {
            match event {
                XmlEvent::StartDocument {
                    version,
                    encoding: _,
                    standalone,
                } => {
                    self.standalone
                        .store(standalone == Some(true), Ordering::Relaxed);
                    if let Some(declaration) = xml_declaration {
                        invoke_handler(
                            vm,
                            &self.xml_decl,
                            (
                                vm.ctx.new_str(version.to_string()),
                                declaration.encoding.map_or_else(
                                    || vm.ctx.none(),
                                    |encoding| vm.ctx.new_str(encoding).into(),
                                ),
                                declaration.standalone.map_or(-1, i32::from),
                            ),
                        )
                    } else {
                        Ok(())
                    }
                }
                XmlEvent::Doctype { .. } => {
                    if let Some(doctype) = doctype {
                        let doctype = *doctype;
                        let has_parameter_entity_reference = doctype.has_parameter_entity_reference;
                        self.external_subset_handled.store(true, Ordering::Relaxed);
                        let parse_entities = self.parse_parameter_entities();
                        let has_external_subset = doctype.system_id.is_some();
                        if has_external_subset
                            && !parse_entities
                            && !self.standalone.load(Ordering::Relaxed)
                            && invoke_int_handler(vm, &self.not_standalone, "NotStandalone", ())?
                                == Some(0)
                        {
                            return Err(self.handler_error(
                                vm,
                                22,
                                "document is not standalone",
                            )?);
                        }
                        let name = self.intern_name(vm, doctype.name);
                        let system_id = doctype.system_id.map(|s| vm.ctx.new_str(s));
                        let public_id = doctype.public_id.map(|s| vm.ctx.new_str(s));
                        invoke_handler(
                            vm,
                            &self.start_doctype_decl,
                            (
                                name,
                                system_id.clone(),
                                public_id.clone(),
                                i32::from(doctype.has_internal_subset),
                            ),
                        )?;
                        for name in &doctype.skipped_parameter_entities {
                            invoke_handler(
                                vm,
                                &self.skipped_entity,
                                (vm.ctx.new_str(name.as_str()), 1),
                            )?;
                        }
                        let mut seen_entity_declarations = std::collections::HashSet::new();
                        for declaration in dtd_declarations(&doctype.syntax) {
                            match declaration {
                                DtdDeclaration::Comment(text) => {
                                    invoke_handler(vm, &self.comment, (vm.ctx.new_str(text),))?;
                                }
                                DtdDeclaration::ProcessingInstruction(target, data) => {
                                    invoke_handler(
                                        vm,
                                        &self.processing_instruction,
                                        (
                                            vm.ctx.new_str(target),
                                            vm.ctx.new_str(
                                                data.trim_start_matches([' ', '\t', '\r', '\n']),
                                            ),
                                        ),
                                    )?;
                                }
                                DtdDeclaration::Element(name, content_model) => {
                                    let model = match content_model.as_str() {
                                        "EMPTY" => content_model_object(
                                            ContentModel {
                                                kind: 1,
                                                quantifier: 0,
                                                name: None,
                                                children: Vec::new(),
                                            },
                                            vm,
                                        ),
                                        "ANY" => content_model_object(
                                            ContentModel {
                                                kind: 2,
                                                quantifier: 0,
                                                name: None,
                                                children: Vec::new(),
                                            },
                                            vm,
                                        ),
                                        _ => {
                                            let Some(model) = element_content_model(&content_model)
                                            else {
                                                if content_model_exceeds_depth(&content_model) {
                                                    return Err(vm.new_recursion_error(
                                                        "content model is nested too deeply",
                                                    ));
                                                }
                                                return Err(self.handler_error(
                                                    vm,
                                                    2,
                                                    "syntax error",
                                                )?);
                                            };
                                            content_model_object(model, vm)
                                        }
                                    };
                                    invoke_handler(
                                        vm,
                                        &self.element_decl,
                                        (vm.ctx.new_str(name), model),
                                    )?;
                                }
                                DtdDeclaration::Attlist(element, name, kind, default, required) => {
                                    let default = default
                                        .map(|value| normalize_attlist_default(&value, &kind));
                                    invoke_handler(
                                        vm,
                                        &self.attlist_decl,
                                        (
                                            vm.ctx.new_str(element),
                                            vm.ctx.new_str(name),
                                            vm.ctx.new_str(kind),
                                            default.map(|value| vm.ctx.new_str(value)),
                                            i32::from(required),
                                        ),
                                    )?
                                }
                                DtdDeclaration::Notation(name, public_id, system_id) => {
                                    let base = self
                                        .base
                                        .read()
                                        .as_ref()
                                        .map(|base| vm.ctx.new_str(base.clone()));
                                    invoke_handler(
                                        vm,
                                        &self.notation_decl,
                                        (
                                            vm.ctx.new_str(name),
                                            base,
                                            system_id.map(|value| vm.ctx.new_str(value)),
                                            public_id.map(|value| vm.ctx.new_str(value)),
                                        ),
                                    )?;
                                }
                                DtdDeclaration::Entity {
                                    name,
                                    parameter,
                                    value,
                                    system_id,
                                    public_id,
                                    notation,
                                } => {
                                    if !seen_entity_declarations.insert((parameter, name.clone())) {
                                        continue;
                                    }
                                    if !parameter
                                        && value.is_none()
                                        && notation.is_none()
                                        && let Some(system_id) = &system_id
                                    {
                                        self.external_entities
                                            .write()
                                            .entry(name.clone())
                                            .or_insert_with(|| {
                                                (Some(system_id.clone()), public_id.clone())
                                            });
                                    }
                                    if notation.is_some() {
                                        let base = self
                                            .base
                                            .read()
                                            .as_ref()
                                            .map(|base| vm.ctx.new_str(base.clone()));
                                        invoke_handler(
                                            vm,
                                            &self.unparsed_entity_decl,
                                            (
                                                vm.ctx.new_str(name),
                                                base,
                                                system_id.map(|value| vm.ctx.new_str(value)),
                                                public_id.map(|value| vm.ctx.new_str(value)),
                                                notation.map(|value| vm.ctx.new_str(value)),
                                            ),
                                        )?;
                                    } else {
                                        let base = self
                                            .base
                                            .read()
                                            .as_ref()
                                            .map(|base| vm.ctx.new_str(base.clone()));
                                        invoke_handler_args(
                                            vm,
                                            &self.entity_decl,
                                            vec![
                                                vm.ctx.new_str(name).into(),
                                                vm.ctx.new_int(i32::from(parameter)).into(),
                                                value
                                                    .map(|value| {
                                                        vm.ctx.new_str(decode_xml_entity_value(
                                                            &value,
                                                        ))
                                                    })
                                                    .map_or_else(|| vm.ctx.none(), Into::into),
                                                base.map_or_else(
                                                    || vm.ctx.none(),
                                                    |base| base.into(),
                                                ),
                                                system_id
                                                    .map(|value| vm.ctx.new_str(value))
                                                    .map_or_else(|| vm.ctx.none(), Into::into),
                                                public_id
                                                    .map(|value| vm.ctx.new_str(value))
                                                    .map_or_else(|| vm.ctx.none(), Into::into),
                                                vm.ctx.none(),
                                            ],
                                        )?;
                                    }
                                }
                            }
                        }
                        if has_parameter_entity_reference
                            && !parse_entities
                            && !self.standalone.load(Ordering::Relaxed)
                            && invoke_int_handler(vm, &self.not_standalone, "NotStandalone", ())?
                                == Some(0)
                        {
                            return Err(self.handler_error(
                                vm,
                                22,
                                "document is not standalone",
                            )?);
                        }
                        if parse_entities
                            && (system_id.is_some() || self.use_foreign_dtd.load(Ordering::Relaxed))
                        {
                            self.handle_external_subset(vm, system_id, public_id)?;
                        }
                        invoke_handler(vm, &self.end_doctype_decl, ())?;
                    }
                    Ok(())
                }
                XmlEvent::StartElement {
                    name,
                    mut attributes,
                    namespace,
                } => {
                    let external_entities = self.external_entities.read();
                    if attributes.iter().any(|attribute| {
                        external_entities
                            .keys()
                            .any(|entity| attribute.value.contains(&entity_marker('E', entity)))
                    }) {
                        return Err(self.handler_error(
                            vm,
                            16,
                            "reference to external entity in attribute",
                        )?);
                    }
                    drop(external_entities);
                    for attribute in &mut attributes {
                        if self.standalone.load(Ordering::Relaxed)
                            && attribute.value.contains("\0S")
                        {
                            return Err(self.handler_error(vm, 11, "undefined entity")?);
                        }
                        attribute.value = remove_skipped_entity_markers(&attribute.value);
                    }
                    if !self.external_subset_handled.swap(true, Ordering::Relaxed)
                        && self.parent.is_none()
                        && self.use_foreign_dtd.load(Ordering::Relaxed)
                        && self.parse_parameter_entities()
                    {
                        self.handle_external_subset(vm, None, None)?;
                    }
                    if self.namespace_separator.is_some() {
                        let declarations = {
                            let mut stack = self.namespace_stack.write();
                            let previous = stack.last().map(|(namespace, _)| namespace);
                            let declarations: Vec<_> = namespace
                                .iter()
                                .filter(|(prefix, uri)| {
                                    previous.map_or_else(
                                        || {
                                            !matches!(
                                                (*prefix, *uri),
                                                ("", "")
                                                    | ("xml", xml::namespace::NS_XML_URI)
                                                    | ("xmlns", xml::namespace::NS_XMLNS_URI)
                                            )
                                        },
                                        |previous| previous.get(prefix) != Some(*uri),
                                    )
                                })
                                .map(|(prefix, uri)| (prefix.to_owned(), uri.to_owned()))
                                .collect();
                            stack.push((
                                namespace,
                                declarations
                                    .iter()
                                    .map(|(prefix, _)| prefix.clone())
                                    .collect(),
                            ));
                            declarations
                        };
                        for (prefix, uri) in declarations {
                            let prefix = (!prefix.is_empty()).then(|| vm.ctx.new_str(prefix));
                            let uri = (!uri.is_empty()).then(|| vm.ctx.new_str(uri));
                            invoke_handler(vm, &self.start_namespace_decl, (prefix, uri))?;
                        }
                    }
                    let ordered = self.ordered_attributes.read().is(&vm.ctx.true_value);
                    // Build the container.
                    let attrs: PyObjectRef = if ordered {
                        let mut items = Vec::with_capacity(attributes.len() * 2);
                        for attribute in attributes {
                            let key = self.intern_name(vm, self.make_name(&attribute.name, vm));
                            items.push(key.into());
                            items.push(vm.ctx.new_str(attribute.value).into());
                        }
                        vm.ctx.new_list(items).into()
                    } else {
                        let dict = vm.ctx.new_dict();
                        for attribute in attributes {
                            let key = self.intern_name(vm, self.make_name(&attribute.name, vm));
                            dict.set_item(
                                AsRef::<str>::as_ref(&key),
                                vm.ctx.new_str(attribute.value).into(),
                                vm,
                            )
                            .unwrap();
                        }
                        dict.into()
                    };

                    let name_str = self.intern_name(vm, self.make_name(&name, vm));
                    invoke_handler(vm, &self.start_element, (name_str, attrs))
                }
                XmlEvent::EndElement { name, .. } => {
                    let name_str = self.intern_name(vm, self.make_name(&name, vm));
                    invoke_handler(vm, &self.end_element, (name_str,))?;
                    let declarations = self.namespace_stack.write().pop();
                    if let Some((_, prefixes)) = declarations {
                        for prefix in prefixes.into_iter().rev() {
                            let prefix = (!prefix.is_empty()).then(|| vm.ctx.new_str(prefix));
                            invoke_handler(vm, &self.end_namespace_decl, (prefix,))?;
                        }
                    }
                    Ok(())
                }
                XmlEvent::Characters(chars) => self.dispatch_character_data(vm, chars),
                XmlEvent::ProcessingInstruction { name, data } => {
                    let name = PyStr::from(name).into_ref(&vm.ctx);
                    let data = data.unwrap_or_default();
                    // Expat skips XML S between the target and data, not trailing whitespace.
                    let data = PyStr::from(data.trim_start_matches([' ', '\t', '\r', '\n']))
                        .into_ref(&vm.ctx);
                    invoke_handler(vm, &self.processing_instruction, (name, data))
                }
                XmlEvent::Comment(comment) => {
                    let comment = PyStr::from(comment).into_ref(&vm.ctx);
                    invoke_handler(vm, &self.comment, (comment,))
                }
                XmlEvent::CData(chars) => {
                    invoke_handler(vm, &self.start_cdata_section, ())?;
                    let str = PyStr::from(chars).into_ref(&vm.ctx);
                    invoke_handler(vm, &self.character_data, (str,))?;
                    invoke_handler(vm, &self.end_cdata_section, ())
                }
                _ => Ok(()),
            }
        }

        /// Tear down the current backend (if any). For `Backend::Threaded`
        /// this stops and joins the parser thread; for `Backend::Sync` it
        /// simply drops the accumulated buffer. Called once the document is
        /// finished, or (for the threaded backend) a handler raised and we
        /// choose not to keep the stream around (see `pump`).
        fn teardown_backend(&self) {
            let mut guard = self.backend.write();
            *guard = None;
        }

        fn parse_parameter_entities(&self) -> bool {
            let mode = self.parameter_entity_parsing.load(Ordering::Relaxed);
            mode == XML_PARAM_ENTITY_PARSING_ALWAYS
                || (mode == XML_PARAM_ENTITY_PARSING_UNLESS_STANDALONE
                    && !self.standalone.load(Ordering::Relaxed))
        }

        fn handle_external_subset(
            &self,
            vm: &VirtualMachine,
            system_id: Option<PyStrRef>,
            public_id: Option<PyStrRef>,
        ) -> PyResult<()> {
            let base = self.base.read().as_ref().map(|s| vm.ctx.new_str(s.clone()));
            let result = invoke_int_handler(
                vm,
                &self.external_entity_ref,
                "ExternalEntityRef",
                (vm.ctx.none(), base, system_id, public_id),
            )?;
            if result == Some(0) {
                return Err(self.handler_error(
                    vm,
                    21,
                    "error in processing external entity reference",
                )?);
            }
            Ok(())
        }

        fn handler_error(
            &self,
            vm: &VirtualMachine,
            code: i32,
            description: &str,
        ) -> PyResult<crate::vm::builtins::PyBaseExceptionRef> {
            let line = *self.current_line.read();
            let column = *self.current_column.read();
            let error = vm.new_exception_msg(
                PyExpatError::class(&vm.ctx).to_owned(),
                format!("{description}: line {line}, column {column}").into(),
            );
            error
                .as_object()
                .set_attr("code", vm.ctx.new_int(code), vm)?;
            error
                .as_object()
                .set_attr("lineno", vm.ctx.new_int(line), vm)?;
            error
                .as_object()
                .set_attr("offset", vm.ctx.new_int(column), vm)?;
            Ok(error)
        }

        fn parse_error(
            &self,
            vm: &VirtualMachine,
            error: xml::reader::Error,
        ) -> PyResult<crate::vm::builtins::PyBaseExceptionRef> {
            let position = error.position();
            let line = position.row() as i64 + 1;
            let column = position.column() as i64;
            *self.current_line.write() = line;
            *self.current_column.write() = column;
            let (code, description) = match error.kind() {
                xml::reader::ErrorKind::UnexpectedEof => (5, "unclosed token"),
                xml::reader::ErrorKind::Syntax(message) => {
                    if message.starts_with("Unexpected end of stream:") {
                        (3, "no element found")
                    } else if message.starts_with("Unexpected end of stream") {
                        (5, "unclosed token")
                    } else if message.starts_with("Unexpected closing tag:") {
                        (7, "mismatched tag")
                    } else if message.starts_with("Attribute ")
                        && message.ends_with(" is redefined")
                    {
                        (8, "duplicate attribute")
                    } else if message.starts_with("Undefined entity:")
                        || message.starts_with("Unexpected entity:")
                    {
                        (11, "undefined entity")
                    } else if message.starts_with("Unclosed <![CDATA[") {
                        (20, "unclosed CDATA section")
                    } else if message.ends_with("prefix is unbound") {
                        (27, "unbound prefix")
                    } else {
                        (2, "syntax error")
                    }
                }
                xml::reader::ErrorKind::Utf8(_) => (4, "not well-formed (invalid token)"),
                _ => (2, "syntax error"),
            };
            let message = format!("{description}: line {line}, column {column}");
            let error =
                vm.new_exception_msg(PyExpatError::class(&vm.ctx).to_owned(), message.into());
            error
                .as_object()
                .set_attr("code", vm.ctx.new_int(code), vm)?;
            error
                .as_object()
                .set_attr("lineno", vm.ctx.new_int(line), vm)?;
            error
                .as_object()
                .set_attr("offset", vm.ctx.new_int(column), vm)?;
            Ok(error)
        }

        /// Drain events already produced by the parser thread (and any
        /// produced while draining), dispatching each to its handler, until
        /// the parser thread reports it is blocked waiting for more input
        /// (`NeedMore`), has finished (`Finished`), or hit a parse error
        /// (`Error`, raised as `ExpatError`). If a handler raises, dispatching stops and
        /// the error propagates, but the parser thread and any events it
        /// already produced are kept around so a later `Parse()` call
        /// resumes exactly where dispatch left off, mirroring libexpat
        /// (a callback exception aborts that `XML_Parse()` call without
        /// corrupting the parser's internal state). Only used by
        /// `Backend::Threaded`.
        #[cfg(not(target_arch = "wasm32"))]
        fn pump(&self, vm: &VirtualMachine, min_pushes: u64) -> PyResult<()> {
            loop {
                let msg = {
                    let guard = self.backend.read();
                    let Some(Backend::Threaded(stream)) = guard.as_ref() else {
                        return Ok(());
                    };
                    let received = stream.rx.lock().unwrap().recv();
                    match received {
                        Ok(msg) => msg,
                        // The parser thread is gone (e.g. it panicked);
                        // nothing more to dispatch.
                        Err(_) => return Ok(()),
                    }
                };
                match msg {
                    ParserMsg::NeedMore(seen) => {
                        if seen >= min_pushes {
                            return Ok(());
                        }
                        // Stale: the parser thread found the buffer empty
                        // and parked *before* the chunk this call just
                        // pushed was visible to it, so it has not consumed
                        // that chunk yet. Returning here would hand control
                        // back to Python having dispatched nothing. The
                        // push already notified the condvar (under the same
                        // mutex the report was sent from, so the wakeup
                        // cannot be lost), so keep draining until the
                        // thread reports on the new data.
                        continue;
                    }
                    ParserMsg::Finished => {
                        self.completed.store(true, Ordering::SeqCst);
                        self.finished.store(true, Ordering::SeqCst);
                        self.teardown_backend();
                        return Ok(());
                    }
                    ParserMsg::Error(error) => {
                        self.teardown_backend();
                        return Err(self.parse_error(vm, error)?);
                    }
                    ParserMsg::Event(ev) => {
                        *self.current_line.write() = ev.line;
                        *self.current_column.write() = ev.column;
                        *self.current_byte_index.write() = ev.byte_index;
                        self.dispatch(vm, ev.event, ev.doctype, ev.xml_declaration)?;
                    }
                }
            }
        }

        /// `Backend::Sync` counterpart of `pump`: grow the accumulated
        /// buffer with `chunk`, re-parse it from scratch, and dispatch only
        /// the events at or beyond `dispatched_events`. See the module doc
        /// comment for why this is O(total bytes fed so far) per call and
        /// only used where `Backend::Threaded` is unavailable.
        ///
        /// Note `EndDocument` reached while re-parsing an as-yet-incomplete
        /// buffer is *not* proof the document is over: a byte slice reader
        /// reports EOF the instant the currently accumulated bytes happen
        /// to form a well-formed document (e.g. right after a root
        /// element's closing tag), even though XML still permits epilog
        /// content (comments, PIs, whitespace) afterwards, and a later
        /// `Parse()` call may append exactly that. So only `isfinal` --
        /// the caller's actual "no more data is coming" signal -- marks the
        /// parser finished here, mirroring `Backend::Threaded` (whose
        /// blocking `Read` only reports EOF once the caller closes the feed
        /// buffer via `isfinal`, so it never reaches `EndDocument`
        /// prematurely).
        fn feed_sync(&self, vm: &VirtualMachine, chunk: &[u8], isfinal: bool) -> PyResult<()> {
            let (config, buffer, skip, parameter_entity_parsing) = {
                let mut guard = self.backend.write();
                let Some(Backend::Sync(sync)) = guard.as_mut() else {
                    // feed_inner only calls feed_sync once the backend has
                    // been initialized to Backend::Sync.
                    return Ok(());
                };
                sync.buffer.extend_from_slice(chunk);
                (
                    sync.config.clone(),
                    sync.buffer.clone(),
                    sync.dispatched_events,
                    sync.parameter_entity_parsing,
                )
            };

            let tracker = Rc::new(RefCell::new(LineTracker::new()));
            let reader = TrackingReader {
                inner: buffer.as_slice(),
                tracker: Rc::clone(&tracker),
            };
            let reader = DoctypeFilterReader::new(reader, parameter_entity_parsing);
            let mut parser = config.create_reader(reader);

            let mut index = 0usize;
            let result = loop {
                match parser.next() {
                    // Deliberately *not* counted towards `index`: on a
                    // buffer that is not yet the whole document, a byte
                    // slice `Read` reports EOF (hence `EndDocument`) the
                    // instant the bytes seen so far happen to form a
                    // well-formed document, e.g. right after a root
                    // element's closing tag -- even though XML still
                    // permits epilog content (comments, PIs, whitespace)
                    // afterwards, and a later `Parse()` call may append
                    // exactly that. If this slot were counted, a later call
                    // that reparses a longer buffer would find a *real*
                    // event at this same index (the epilog content) and
                    // wrongly skip it as already dispatched. Real events
                    // never move to an earlier index across reparses (the
                    // bytes producing them are unchanged), so leaving
                    // `EndDocument` uncounted is always safe.
                    Ok(XmlEvent::EndDocument) => break Ok(()),
                    Ok(event) => {
                        if let XmlEvent::Doctype { syntax } = &event {
                            let references = parser.source().entity_references();
                            let replacements = dtd_entity_replacements(syntax, references);
                            let _ = parser.add_entities(replacements);
                        }
                        let xml_declaration = parser.source().xml_declaration();
                        let this_index = index;
                        index += 1;
                        let pos = parser.position();
                        let byte_index = tracker.borrow_mut().byte_index_for(pos);
                        *self.current_line.write() = pos.row() as i64 + 1;
                        *self.current_column.write() = pos.column() as i64;
                        *self.current_byte_index.write() = byte_index;
                        if this_index < skip {
                            continue;
                        }
                        let doctype = Doctype::from_reader(
                            &parser,
                            &event,
                            parser.source().has_parameter_entity_reference(),
                            parser.source().skipped_parameter_entities(),
                        )
                        .map(Box::new);
                        if let Err(e) = self.dispatch(vm, event, doctype, xml_declaration) {
                            // The handler for `this_index` was invoked (it
                            // raised, but it ran), so don't replay it next
                            // call -- mirrors `Backend::Threaded`, where an
                            // event already popped off the channel is gone
                            // for good.
                            index = this_index + 1;
                            break Err(e);
                        }
                    }
                    // A non-final buffer can end in the middle of a token;
                    // defer only that incomplete-input error until more data
                    // arrives. Malformed complete input must still fail.
                    Err(error) if !isfinal && is_incomplete(&error) => break Ok(()),
                    Err(error) => break Err(self.parse_error(vm, error)?),
                }
            };

            {
                let mut guard = self.backend.write();
                if let Some(Backend::Sync(sync)) = guard.as_mut() {
                    sync.dispatched_events = index.max(skip);
                }
            }
            if isfinal {
                // Remember successful completion separately from errors;
                // both close this backend, but only success means the parser
                // should report XML_ERROR_FINISHED on a later call.
                if result.is_ok() {
                    self.completed.store(true, Ordering::SeqCst);
                }
                self.finished.store(true, Ordering::SeqCst);
                self.teardown_backend();
            }
            result
        }

        fn check_completed(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.completed.load(Ordering::SeqCst) {
                return Err(self.handler_error(vm, 36, "parsing finished")?);
            }
            Ok(())
        }

        /// Feed another chunk of the document to the (possibly newly
        /// created) backend and dispatch whatever events that unblocks.
        /// `Parse()` and `ParseFile()` may each be called several times
        /// with successive chunks of one logical document (e.g.
        /// `xml.etree.ElementTree` reads and feeds a file 64KB at a time);
        /// see the module doc comment for how each backend stays consistent
        /// across those calls.
        fn feed(&self, vm: &VirtualMachine, chunk: &[u8], isfinal: bool) -> PyResult<()> {
            if self.busy.swap(true, Ordering::SeqCst) {
                // Reentrant call, e.g. a handler calling Parse() on the same
                // parser. Draining our own channel (Threaded) or re-entering
                // feed_sync (Sync) from within itself would deadlock or
                // corrupt state, so reject it instead, mirroring CPython.
                return Err(vm.new_runtime_error("Parse() called before Parse() returned"));
            }
            let result = self.feed_inner(vm, chunk, isfinal);
            self.busy.store(false, Ordering::SeqCst);
            result
        }

        fn feed_inner(&self, vm: &VirtualMachine, chunk: &[u8], isfinal: bool) -> PyResult<()> {
            if let Some(entity) = &self.external_entity {
                // xml-rs only parses complete documents. In particular, an
                // external DTD is not a document, and a general external
                // entity can contain multiple top-level nodes and uses the
                // namespace/entity context supplied by the parent. Keep
                // construction and inherited configuration available, but
                // never silently treat those inputs as ordinary documents.
                let kind = if entity.context.is_none() {
                    "external DTD"
                } else {
                    "external entity fragment"
                };
                let encoding = if entity.encoding.is_some() {
                    " with an explicit encoding"
                } else {
                    ""
                };
                return Err(vm.new_not_implemented_error(format!(
                    "{kind} parsing{encoding} is not supported by the xml-rs backend"
                )));
            }
            self.started.store(true, Ordering::Relaxed);
            self.check_completed(vm)?;
            if self.finished.load(Ordering::SeqCst) {
                return Ok(());
            }
            {
                let mut guard = self.backend.write();
                if guard.is_none() {
                    *guard = Some(Backend::new(
                        self.create_config(),
                        self.parameter_entity_parsing.load(Ordering::Relaxed),
                    ));
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let is_threaded = matches!(*self.backend.read(), Some(Backend::Threaded(_)));
                if is_threaded {
                    if post_fork_child() {
                        // This parser's `Backend::Threaded` was created
                        // before a fork this process has since undergone
                        // (a brand new parser created *after* the fork
                        // would have gone through `Backend::new` ->
                        // `ParserStream::try_spawn`, which itself refuses
                        // to spawn once `post_fork_child()` and falls back
                        // to `Backend::Sync` instead of ever reaching this
                        // branch). See the module doc comment: its
                        // `FeedBuffer` mutex/condvar may look held by a
                        // thread that no longer exists in this process, so
                        // touching it (even just to push more data) is
                        // unsafe. Tear it down the same safe way `Drop`
                        // does (see `ParserStream::drop`) and fail this
                        // call clearly instead, mirroring how libexpat
                        // itself has no notion of surviving a fork
                        // mid-parse.
                        self.teardown_backend();
                        self.finished.store(true, Ordering::SeqCst);
                        return Err(vm.new_exception_msg(
                            PyExpatError::class(&vm.ctx).to_owned(),
                            "cannot continue parsing across a fork()".to_owned().into(),
                        ));
                    }
                    let pushes = {
                        let guard = self.backend.read();
                        match guard.as_ref() {
                            Some(Backend::Threaded(stream)) => stream.buf.push(chunk, isfinal),
                            _ => return Ok(()),
                        }
                    };
                    return self.pump(vm, pushes);
                }
            }
            self.feed_sync(vm, chunk, isfinal)
        }

        #[pymethod(name = "Parse")]
        fn parse(
            zelf: &Py<Self>,
            data: ArgStrOrBytesLike,
            isfinal: OptionalArg<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<i32> {
            let bytes = data.borrow_bytes().to_vec();
            zelf.feed(vm, &bytes, isfinal.unwrap_or(false))?;
            Ok(1)
        }

        #[pymethod(name = "ParseFile")]
        fn parse_file(zelf: &Py<Self>, file: PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
            let read = vm
                .get_attribute_opt(&file, "read")?
                .ok_or_else(|| vm.new_type_error("argument must have 'read' attribute"))?;
            zelf.check_completed(vm)?;
            let read_res = read.call((), vm)?;
            let bytes_like = ArgBytesLike::try_from_object(vm, read_res)?;
            let buf = bytes_like.borrow_buf().to_vec();
            // `file.read()` with no argument reads to EOF, so this chunk is
            // always the last one.
            zelf.feed(vm, &buf, true)?;
            Ok(1)
        }
    }

    #[derive(FromArgs)]
    struct ParserCreateArgs {
        #[pyarg(any, optional)]
        encoding: Option<PyStrRef>,
        #[pyarg(any, optional)]
        namespace_separator: Option<PyUtf8StrRef>,
        #[pyarg(any, optional)]
        intern: OptionalOption<PyObjectRef>,
    }

    #[pyfunction(name = "ParserCreate")]
    fn parser_create(
        args: ParserCreateArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyExpatLikeXmlParserRef> {
        // Validate namespace_separator: must be at most one character
        let ns_sep = match args.namespace_separator {
            Some(ref s) => {
                if s.as_str().chars().count() > 1 {
                    return Err(vm.new_value_error(
                        "namespace_separator must be at most one character, omitted, or None",
                    ));
                }
                Some(s.as_str().to_owned())
            }
            None => None,
        };

        // encoding parameter is currently not used (xml-rs handles encoding from XML declaration)
        let _ = args.encoding;

        Ok(PyExpatLikeXmlParser::new(
            ns_sep,
            args.intern,
            None,
            None,
            vm,
        ))
    }

    // TODO: Tie this exception to the module's state.
    #[pyattr]
    #[pyattr(name = "error")]
    #[pyexception(name = "ExpatError", module = "xml.parsers.expat", base = PyException)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(super) struct PyExpatError(PyException);

    #[pyexception]
    impl PyExpatError {}
}

#[pymodule(name = "model")]
mod _model {}

#[pymodule(name = "pyexpat.errors")]
mod _errors {
    use crate::vm::{Py, PyResult, VirtualMachine, builtins::PyModule, extend_module};

    // CPython 3.15 Modules/pyexpat.c error_info_of; code zero has no message.
    pub(super) const ERRORS: &[(&str, &str)] = &[
        ("XML_ERROR_NO_MEMORY", "out of memory"),
        ("XML_ERROR_SYNTAX", "syntax error"),
        ("XML_ERROR_NO_ELEMENTS", "no element found"),
        ("XML_ERROR_INVALID_TOKEN", "not well-formed (invalid token)"),
        ("XML_ERROR_UNCLOSED_TOKEN", "unclosed token"),
        ("XML_ERROR_PARTIAL_CHAR", "partial character"),
        ("XML_ERROR_TAG_MISMATCH", "mismatched tag"),
        ("XML_ERROR_DUPLICATE_ATTRIBUTE", "duplicate attribute"),
        (
            "XML_ERROR_JUNK_AFTER_DOC_ELEMENT",
            "junk after document element",
        ),
        (
            "XML_ERROR_PARAM_ENTITY_REF",
            "illegal parameter entity reference",
        ),
        ("XML_ERROR_UNDEFINED_ENTITY", "undefined entity"),
        (
            "XML_ERROR_RECURSIVE_ENTITY_REF",
            "recursive entity reference",
        ),
        ("XML_ERROR_ASYNC_ENTITY", "asynchronous entity"),
        (
            "XML_ERROR_BAD_CHAR_REF",
            "reference to invalid character number",
        ),
        ("XML_ERROR_BINARY_ENTITY_REF", "reference to binary entity"),
        (
            "XML_ERROR_ATTRIBUTE_EXTERNAL_ENTITY_REF",
            "reference to external entity in attribute",
        ),
        (
            "XML_ERROR_MISPLACED_XML_PI",
            "XML or text declaration not at start of entity",
        ),
        ("XML_ERROR_UNKNOWN_ENCODING", "unknown encoding"),
        (
            "XML_ERROR_INCORRECT_ENCODING",
            "encoding specified in XML declaration is incorrect",
        ),
        ("XML_ERROR_UNCLOSED_CDATA_SECTION", "unclosed CDATA section"),
        (
            "XML_ERROR_EXTERNAL_ENTITY_HANDLING",
            "error in processing external entity reference",
        ),
        ("XML_ERROR_NOT_STANDALONE", "document is not standalone"),
        (
            "XML_ERROR_UNEXPECTED_STATE",
            "unexpected parser state - please send a bug report",
        ),
        (
            "XML_ERROR_ENTITY_DECLARED_IN_PE",
            "entity declared in parameter entity",
        ),
        (
            "XML_ERROR_FEATURE_REQUIRES_XML_DTD",
            "requested feature requires XML_DTD support in Expat",
        ),
        (
            "XML_ERROR_CANT_CHANGE_FEATURE_ONCE_PARSING",
            "cannot change setting once parsing has begun",
        ),
        ("XML_ERROR_UNBOUND_PREFIX", "unbound prefix"),
        ("XML_ERROR_UNDECLARING_PREFIX", "must not undeclare prefix"),
        (
            "XML_ERROR_INCOMPLETE_PE",
            "incomplete markup in parameter entity",
        ),
        ("XML_ERROR_XML_DECL", "XML declaration not well-formed"),
        ("XML_ERROR_TEXT_DECL", "text declaration not well-formed"),
        ("XML_ERROR_PUBLICID", "illegal character(s) in public id"),
        ("XML_ERROR_SUSPENDED", "parser suspended"),
        ("XML_ERROR_NOT_SUSPENDED", "parser not suspended"),
        ("XML_ERROR_ABORTED", "parsing aborted"),
        ("XML_ERROR_FINISHED", "parsing finished"),
        (
            "XML_ERROR_SUSPEND_PE",
            "cannot suspend in external parameter entity",
        ),
        (
            "XML_ERROR_RESERVED_PREFIX_XML",
            "reserved prefix (xml) must not be undeclared or bound to another namespace name",
        ),
        (
            "XML_ERROR_RESERVED_PREFIX_XMLNS",
            "reserved prefix (xmlns) must not be declared or undeclared",
        ),
        (
            "XML_ERROR_RESERVED_NAMESPACE_URI",
            "prefix must not be bound to one of the reserved namespace names",
        ),
        ("XML_ERROR_INVALID_ARGUMENT", "invalid argument"),
        (
            "XML_ERROR_NO_BUFFER",
            "a successful prior call to function XML_GetBuffer is required",
        ),
        (
            "XML_ERROR_AMPLIFICATION_LIMIT_BREACH",
            "limit on input amplification factor (from DTD and entities) breached",
        ),
        ("XML_ERROR_NOT_STARTED", "parser not started"),
    ];

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        let codes = vm.ctx.new_dict();
        let messages = vm.ctx.new_dict();
        for (index, &(name, message)) in ERRORS.iter().enumerate() {
            let code = vm.ctx.new_int(index + 1);
            module.set_attr(name, vm.ctx.new_str(message), vm)?;
            codes.set_item(message, code.clone().into(), vm)?;
            messages.set_item(&(index + 1), vm.ctx.new_str(message).into(), vm)?;
        }
        extend_module!(vm, module, {
            "__doc__" => vm.ctx.new_str("Constants used to describe error conditions."),
            "codes" => codes,
            "messages" => messages,
        });
        Ok(())
    }
}
