//! Pyexpat builtin module
//!
//! ## Incremental parsing / threading protocol
//!
//! `xml.etree.ElementTree`, `xml.sax`, and similar callers feed a document to
//! `Parse()`/`ParseFile()` in many small chunks (e.g. 64KB at a time) rather
//! than all at once, and expect the parser to retain state across those
//! calls the way libexpat's `XML_Parse` does. `xml-rs`'s `EventReader` has no
//! API to pause and resume across separate `Read` sources, so instead one
//! `EventReader` is driven to completion on a dedicated background thread,
//! fed through a `Read` implementation (`FeedReader`) backed by a shared
//! byte queue (`FeedBuffer`). This keeps the per-`Parse()` cost proportional
//! to the size of that call's chunk, rather than to the whole document seen
//! so far.
//!
//! Protocol, implemented by `ParserStream`/`FeedBuffer`/`run_parser_thread`:
//! - `ParserCreate()` does not spawn a thread; one is spawned lazily by the
//!   first `Parse()`/`ParseFile()` call (`PyExpatLikeXmlParser::feed_inner`),
//!   so a parser created and dropped without being fed leaks nothing.
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
//!   instead.
//! - Dropping the parser (`ParserStream::drop`) always stops (`FeedBuffer`
//!   is marked stopped, waking any blocked `read()`) and joins the thread,
//!   so no thread is ever leaked, whether or not the parser was ever fed to
//!   completion.

// spell-checker: ignore libexpat

pub(crate) use _pyexpat::module_def;

macro_rules! create_property {
    ($ctx: expr, $attributes: expr, $name: expr, $class: expr, $element: ident) => {
        let attr = $ctx.new_static_getset(
            $name,
            $class,
            move |this: &PyExpatLikeXmlParser| this.$element.read().clone(),
            move |this: &PyExpatLikeXmlParser, func: PyObjectRef| *this.$element.write() = func,
        );

        $attributes.insert($ctx.intern_str($name), attr.into());
    };
}

macro_rules! create_readonly_int_property {
    ($ctx: expr, $attributes: expr, $name: expr, $class: expr, $element: ident) => {
        let getset = crate::vm::builtins::PyGetSet::new($name, $class).with_get(
            move |this: &PyExpatLikeXmlParser, vm: &VirtualMachine| -> PyObjectRef {
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
            move |this: &PyExpatLikeXmlParser| this.$element.read().clone(),
            move |this: &PyExpatLikeXmlParser,
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
        builtins::{PyBytesRef, PyException, PyModule, PyStr, PyStrRef, PyType, PyUtf8StrRef},
        extend_module,
        function::{ArgBytesLike, ArgPrimitiveIndex, Either, IntoFuncArgs, OptionalArg},
        types::Constructor,
    };
    use alloc::collections::VecDeque;
    use alloc::rc::Rc;
    use alloc::sync::Arc;
    use core::cell::RefCell;
    use core::sync::atomic::{AtomicBool, Ordering};
    use rustpython_common::lock::PyRwLock;
    use std::io::{BufReader, Read};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Condvar, Mutex};
    use std::thread::JoinHandle;
    use xml::common::Position;
    use xml::reader::XmlEvent;

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);

        // Add submodules
        let model = super::_model::module_def(&vm.ctx).create_module(vm)?;
        let errors = super::_errors::module_def(&vm.ctx).create_module(vm)?;

        extend_module!(vm, module, {
            "model" => model,
            "errors" => errors,
        });

        Ok(())
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
    #[pyclass(name = "xmlparser", module = false, traverse)]
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
        // See the module-level `//!` doc comment for the full threading
        // protocol; `stream` is `None` until the first `Parse()`/`ParseFile()`
        // call, so a parser that is created and dropped without ever being
        // fed never spawns a thread.
        #[pytraverse(skip)]
        stream: PyRwLock<Option<ParserStream>>,
        // Guards against Parse()/ParseFile() being invoked reentrantly (e.g.
        // a handler calling Parse() on its own parser), which would
        // otherwise deadlock waiting on a channel this same call is meant to
        // drain.
        #[pytraverse(skip)]
        busy: AtomicBool,
        // Set once EndDocument has been observed, so further Parse() calls
        // become harmless no-ops instead of starting a new document.
        #[pytraverse(skip)]
        finished: AtomicBool,
        start_element: MutableObject,
        end_element: MutableObject,
        character_data: MutableObject,
        entity_decl: MutableObject,
        buffer_text: MutableObject,
        namespace_prefixes: MutableObject,
        ordered_attributes: MutableObject,
        specified_attributes: MutableObject,
        intern: MutableObject,
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
            for &b in bytes {
                self.current_line.push_back(b);
                self.total_read += 1;
                if b == b'\n' {
                    self.line_starts.push(self.total_read);
                    self.current_line.clear();
                    self.line_scanned_bytes = 0;
                    self.line_scanned_chars = 0;
                }
            }
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
                self.current_line.make_contiguous();
                let (slice, _) = self.current_line.as_slices();
                let text = core::str::from_utf8(slice).unwrap_or("");
                let mut consumed_bytes = 0usize;
                let mut consumed_chars = 0usize;
                for ch in text.chars().take(take) {
                    consumed_bytes += ch.len_utf8();
                    consumed_chars += 1;
                }
                for _ in 0..consumed_bytes {
                    self.current_line.pop_front();
                }
                self.line_scanned_bytes += consumed_bytes;
                self.line_scanned_chars += consumed_chars;
            }
            (line_start + self.line_scanned_bytes) as i64
        }
    }

    /// Shared feed buffer between the calling (Python) thread and the parser
    /// thread. See the module-level doc comment for the full protocol.
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
    }

    struct FeedBuffer {
        inner: Mutex<FeedInner>,
        cv: Condvar,
    }

    impl FeedBuffer {
        fn new() -> Self {
            Self {
                inner: Mutex::new(FeedInner {
                    data: VecDeque::new(),
                    closed: false,
                    stopped: false,
                }),
                cv: Condvar::new(),
            }
        }

        fn push(&self, chunk: &[u8], isfinal: bool) {
            let mut inner = self.inner.lock().unwrap();
            inner.data.extend(chunk.iter().copied());
            if isfinal {
                inner.closed = true;
            }
            self.cv.notify_all();
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
    struct FeedReader {
        buf: Arc<FeedBuffer>,
        tracker: Rc<RefCell<LineTracker>>,
        events_tx: Sender<ParserMsg>,
    }

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
                let _ = self.events_tx.send(ParserMsg::NeedMore);
                inner = self.buf.cv.wait(inner).unwrap();
            }
        }
    }

    /// One parser event, prepared on the parser thread (position captured
    /// there, since only that thread drives the `EventReader`) and handed to
    /// the calling thread for dispatch.
    struct PreparedEvent {
        event: XmlEvent,
        line: i64,
        column: i64,
        byte_index: i64,
    }

    enum ParserMsg {
        /// The parser thread's `Read` call is blocked waiting for more
        /// input: everything fed so far has been turned into events (or
        /// `NeedMore`/`Event` messages already sent), so the calling thread
        /// can safely return control to Python.
        NeedMore,
        Event(PreparedEvent),
        /// `EndDocument` was reached; the parser thread has exited.
        Finished,
        /// xml-rs reported a parse error. Mirrors the previous
        /// whole-buffer implementation, which silently stopped dispatching
        /// on the first parse error rather than raising `ExpatError`.
        Error,
    }

    /// A live parser thread plus the channel it reports through. Dropping
    /// this stops the thread (via `FeedBuffer::stop`, which makes its
    /// blocked or future `read()` calls return EOF) and joins it, so a
    /// parser can never leak a thread, whether or not it was ever fully fed.
    struct ParserStream {
        buf: Arc<FeedBuffer>,
        rx: Mutex<Receiver<ParserMsg>>,
        thread: Option<JoinHandle<()>>,
    }

    impl core::fmt::Debug for ParserStream {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("ParserStream").finish_non_exhaustive()
        }
    }

    impl ParserStream {
        fn spawn(config: xml::ParserConfig) -> Self {
            let buf = Arc::new(FeedBuffer::new());
            let (tx, rx) = mpsc::channel();
            let thread_buf = Arc::clone(&buf);
            let thread = std::thread::Builder::new()
                .name("pyexpat-parser".into())
                .spawn(move || run_parser_thread(config, thread_buf, tx))
                .expect("failed to spawn pyexpat parser thread");
            Self {
                buf,
                rx: Mutex::new(rx),
                thread: Some(thread),
            }
        }
    }

    impl Drop for ParserStream {
        fn drop(&mut self) {
            self.buf.stop();
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
        }
    }

    /// Body of the parser thread: drives a single long-lived `EventReader`
    /// over the `FeedBuffer`-backed `Read`, sending every event (and
    /// position/EOF/error notifications) back over `tx`. Never touches the
    /// VM or calls into Python -- handlers are dispatched by the calling
    /// thread from the messages sent here, exactly as the module doc
    /// comment requires.
    fn run_parser_thread(config: xml::ParserConfig, buf: Arc<FeedBuffer>, tx: Sender<ParserMsg>) {
        let tracker = Rc::new(RefCell::new(LineTracker::new()));
        let reader = FeedReader {
            buf,
            tracker: Rc::clone(&tracker),
            events_tx: tx.clone(),
        };
        let mut parser = config.create_reader(BufReader::new(reader));
        loop {
            match parser.next() {
                Ok(XmlEvent::EndDocument) => {
                    let _ = tx.send(ParserMsg::Finished);
                    break;
                }
                Ok(event) => {
                    let pos = parser.position();
                    let byte_index = tracker.borrow_mut().byte_index_for(pos);
                    let msg = ParserMsg::Event(PreparedEvent {
                        event,
                        line: pos.row() as i64 + 1,
                        column: pos.column() as i64,
                        byte_index,
                    });
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(ParserMsg::Error);
                    break;
                }
            }
        }
    }

    #[pyclass]
    impl PyExpatLikeXmlParser {
        fn new(
            namespace_separator: Option<String>,
            intern: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyExpatLikeXmlParserRef {
            let intern_dict = intern.unwrap_or_else(|| vm.ctx.new_dict().into());
            Self {
                namespace_separator,
                base: PyRwLock::new(None),
                current_line: PyRwLock::new(1),
                current_column: PyRwLock::new(0),
                current_byte_index: PyRwLock::new(-1),
                stream: PyRwLock::new(None),
                busy: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                start_element: MutableObject::new(vm.ctx.none()),
                end_element: MutableObject::new(vm.ctx.none()),
                character_data: MutableObject::new(vm.ctx.none()),
                entity_decl: MutableObject::new(vm.ctx.none()),
                buffer_text: MutableObject::new(vm.ctx.new_bool(false).into()),
                namespace_prefixes: MutableObject::new(vm.ctx.new_bool(false).into()),
                ordered_attributes: MutableObject::new(vm.ctx.new_bool(false).into()),
                specified_attributes: MutableObject::new(vm.ctx.new_bool(false).into()),
                intern: MutableObject::new(intern_dict),
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
            create_property!(ctx, attributes, "intern", class, intern);
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

        #[pymethod(name = "SetParamEntityParsing")]
        fn set_param_entity_parsing(&self, _flag: ArgPrimitiveIndex<i32>) -> i32 {
            // Compatibility shim: xml.sax requires this setup API, but xml-rs
            // does not expose Expat parameter entity parsing configuration.
            1
        }

        #[pymethod(name = "UseForeignDTD")]
        fn use_foreign_dtd(&self, _flag: OptionalArg<bool>) {
            // Compatibility shim: CPython's implementation forwards the flag to
            // libexpat's XML_UseForeignDTD, which lets a DTD handler splice in an
            // external subset for documents that only declare one (e.g. via
            // NotStandaloneHandler). The xml-rs backend used here has no such hook
            // and always parses documents standalone, so the flag is accepted and
            // ignored purely so callers that toggle this setting (e.g. genshi) don't
            // fail with AttributeError.
        }

        #[pymethod(name = "SetBase")]
        fn set_base(&self, base: PyStrRef) {
            // Store-only compatibility state for xml.sax locator APIs. The
            // xml-rs backend still does not perform Expat-style base URI
            // resolution for external entities.
            *self.base.write() = Some(AsRef::<str>::as_ref(&base).to_owned());
        }

        #[pymethod(name = "GetBase")]
        fn get_base(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.base.read().as_ref().map_or_else(
                || vm.ctx.none(),
                |base| vm.ctx.new_str(base.as_str()).into(),
            )
        }

        /// Construct element name with namespace if separator is set
        fn make_name(&self, name: &xml::name::OwnedName) -> String {
            match (&self.namespace_separator, &name.namespace) {
                (Some(sep), Some(ns)) => format!("{}{}{}", ns, sep, name.local_name),
                _ => name.local_name.clone(),
            }
        }

        /// Dispatch a single parser event to the registered handlers. Runs on
        /// the calling thread only -- see the module doc comment.
        fn dispatch(&self, vm: &VirtualMachine, event: XmlEvent) -> PyResult<()> {
            match event {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    let ordered = self.ordered_attributes.read().is(&vm.ctx.true_value);
                    // Build the container.
                    let attrs: PyObjectRef = if ordered {
                        let mut items = Vec::with_capacity(attributes.len() * 2);
                        for attribute in attributes {
                            items.push(vm.ctx.new_str(self.make_name(&attribute.name)).into());
                            items.push(vm.ctx.new_str(attribute.value).into());
                        }
                        vm.ctx.new_list(items).into()
                    } else {
                        let dict = vm.ctx.new_dict();
                        for attribute in attributes {
                            dict.set_item(
                                self.make_name(&attribute.name).as_str(),
                                vm.ctx.new_str(attribute.value).into(),
                                vm,
                            )
                            .unwrap();
                        }
                        dict.into()
                    };

                    let name_str = PyStr::from(self.make_name(&name)).into_ref(&vm.ctx);
                    invoke_handler(vm, &self.start_element, (name_str, attrs))
                }
                XmlEvent::EndElement { name, .. } => {
                    let name_str = PyStr::from(self.make_name(&name)).into_ref(&vm.ctx);
                    invoke_handler(vm, &self.end_element, (name_str,))
                }
                XmlEvent::Characters(chars) => {
                    let str = PyStr::from(chars).into_ref(&vm.ctx);
                    invoke_handler(vm, &self.character_data, (str,))
                }
                XmlEvent::ProcessingInstruction { name, data } => {
                    let name = PyStr::from(name).into_ref(&vm.ctx);
                    let data = PyStr::from(data.unwrap_or_default()).into_ref(&vm.ctx);
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

        /// Tear down the current parser thread (if any), stopping and
        /// joining it. Called once the document is finished, or a handler
        /// raised and we choose not to keep the stream around (see `pump`).
        fn teardown_stream(&self) {
            let mut guard = self.stream.write();
            *guard = None;
        }

        /// Drain events already produced by the parser thread (and any
        /// produced while draining), dispatching each to its handler, until
        /// the parser thread reports it is blocked waiting for more input
        /// (`NeedMore`), has finished (`Finished`), or hit a parse error
        /// (`Error`, silently swallowed for compatibility -- see
        /// `ParserMsg::Error`). If a handler raises, dispatching stops and
        /// the error propagates, but the parser thread and any events it
        /// already produced are kept around so a later `Parse()` call
        /// resumes exactly where dispatch left off, mirroring libexpat
        /// (a callback exception aborts that `XML_Parse()` call without
        /// corrupting the parser's internal state).
        fn pump(&self, vm: &VirtualMachine) -> PyResult<()> {
            loop {
                let msg = {
                    let guard = self.stream.read();
                    let Some(stream) = guard.as_ref() else {
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
                    ParserMsg::NeedMore => return Ok(()),
                    ParserMsg::Finished => {
                        self.finished.store(true, Ordering::SeqCst);
                        self.teardown_stream();
                        return Ok(());
                    }
                    ParserMsg::Error => {
                        self.teardown_stream();
                        return Ok(());
                    }
                    ParserMsg::Event(ev) => {
                        *self.current_line.write() = ev.line;
                        *self.current_column.write() = ev.column;
                        *self.current_byte_index.write() = ev.byte_index;
                        self.dispatch(vm, ev.event)?;
                    }
                }
            }
        }

        /// Feed another chunk of the document to the (possibly newly
        /// spawned) parser thread and dispatch whatever events that
        /// unblocks. `Parse()` and `ParseFile()` may each be called several
        /// times with successive chunks of one logical document (e.g.
        /// `xml.etree.ElementTree` reads and feeds a file 64KB at a time);
        /// see the module doc comment for how a single parser thread stays
        /// alive across those calls.
        fn feed(&self, vm: &VirtualMachine, chunk: &[u8], isfinal: bool) -> PyResult<()> {
            if self.busy.swap(true, Ordering::SeqCst) {
                // Reentrant call, e.g. a handler calling Parse() on the same
                // parser. Draining our own channel from within itself would
                // deadlock, so reject it instead, mirroring CPython.
                return Err(vm.new_runtime_error("Parse() called before Parse() returned"));
            }
            let result = self.feed_inner(vm, chunk, isfinal);
            self.busy.store(false, Ordering::SeqCst);
            result
        }

        fn feed_inner(&self, vm: &VirtualMachine, chunk: &[u8], isfinal: bool) -> PyResult<()> {
            if self.finished.load(Ordering::SeqCst) {
                // The document already ended; further data is ignored, same
                // as libexpat happily no-oping once XML_Parse has seen EOF.
                return Ok(());
            }
            {
                let mut guard = self.stream.write();
                if guard.is_none() {
                    *guard = Some(ParserStream::spawn(self.create_config()));
                }
            }
            {
                let guard = self.stream.read();
                guard.as_ref().unwrap().buf.push(chunk, isfinal);
            }
            self.pump(vm)
        }

        #[pymethod(name = "Parse")]
        fn parse(
            &self,
            data: Either<PyStrRef, PyBytesRef>,
            isfinal: OptionalArg<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<i32> {
            let bytes = match data {
                Either::A(s) => s.as_bytes().to_vec(),
                Either::B(b) => b.as_bytes().to_vec(),
            };
            self.feed(vm, &bytes, isfinal.unwrap_or(false))?;
            Ok(1)
        }

        #[pymethod(name = "ParseFile")]
        fn parse_file(&self, file: PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
            let read_res = vm.call_method(&file, "read", ())?;
            let bytes_like = ArgBytesLike::try_from_object(vm, read_res)?;
            let buf = bytes_like.borrow_buf().to_vec();
            // `file.read()` with no argument reads to EOF, so this chunk is
            // always the last one.
            self.feed(vm, &buf, true)?;
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
        intern: Option<PyObjectRef>,
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

        Ok(PyExpatLikeXmlParser::new(ns_sep, args.intern, vm))
    }

    // TODO: Tie this exception to the module's state.
    #[pyattr]
    #[pyattr(name = "error")]
    #[pyexception(name = "ExpatError", base = PyException)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(super) struct PyExpatError(PyException);

    #[pyexception]
    impl PyExpatError {}
}

#[pymodule(name = "model")]
mod _model {}

#[pymodule(name = "errors")]
mod _errors {}
