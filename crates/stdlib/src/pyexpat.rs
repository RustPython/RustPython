/// Pyexpat builtin module
use crate::vm::{PyRef, VirtualMachine, builtins::PyModule, extend_module};

pub fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _pyexpat::make_module(vm);

    extend_module!(vm, &module, {
         "errors" => _errors::make_module(vm),
         "model" => _model::make_module(vm),
    });

    module
}

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
        Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
        builtins::{PyStr, PyStrRef, PyType},
        function::ArgBytesLike,
        function::{IntoFuncArgs, OptionalArg},
    };
    use rustpython_common::lock::PyRwLock;
    use std::io::Cursor;
    use xml::reader::XmlEvent;

    type MutableObject = PyRwLock<PyObjectRef>;

    #[pyattr(name = "version_info")]
    pub const VERSION_INFO: (u32, u32, u32) = (2, 7, 1);

    #[pyattr]
    #[pyclass(name = "xmlparser", module = false, traverse)]
    #[derive(Debug, PyPayload)]
    pub struct PyExpatLikeXmlParser {
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
    fn invoke_handler<T>(vm: &VirtualMachine, handler: &MutableObject, args: T)
    where
        T: IntoFuncArgs,
    {
        // Clone the handler while holding the read lock, then release the lock
        let handler = handler.read().clone();
        handler.call(args, vm).ok();
    }

    #[pyclass]
    impl PyExpatLikeXmlParser {
        fn new(vm: &VirtualMachine) -> PyResult<PyExpatLikeXmlParserRef> {
            Ok(Self {
                start_element: MutableObject::new(vm.ctx.none()),
                end_element: MutableObject::new(vm.ctx.none()),
                character_data: MutableObject::new(vm.ctx.none()),
                entity_decl: MutableObject::new(vm.ctx.none()),
                buffer_text: MutableObject::new(vm.ctx.new_bool(false).into()),
                namespace_prefixes: MutableObject::new(vm.ctx.new_bool(false).into()),
                ordered_attributes: MutableObject::new(vm.ctx.new_bool(false).into()),
                specified_attributes: MutableObject::new(vm.ctx.new_bool(false).into()),
                // String interning dictionary - used by the parser to intern element/attribute names
                // for memory efficiency and faster comparisons. See CPython's pyexpat documentation.
                intern: MutableObject::new(vm.ctx.new_dict().into()),
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
            .into_ref(&vm.ctx))
        }

        #[extend_class]
        fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
            let mut attributes = class.attributes.write();

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
                .cdata_to_characters(true)
                .coalesce_characters(false)
                .whitespace_to_characters(true)
        }

        fn do_parse<T>(&self, vm: &VirtualMachine, parser: xml::EventReader<T>)
        where
            T: std::io::Read,
        {
            for e in parser {
                match e {
                    Ok(XmlEvent::StartElement {
                        name, attributes, ..
                    }) => {
                        let dict = vm.ctx.new_dict();
                        for attribute in attributes {
                            dict.set_item(
                                attribute.name.local_name.as_str(),
                                vm.ctx.new_str(attribute.value).into(),
                                vm,
                            )
                            .unwrap();
                        }

                        let name_str = PyStr::from(name.local_name).into_ref(&vm.ctx);
                        invoke_handler(vm, &self.start_element, (name_str, dict));
                    }
                    Ok(XmlEvent::EndElement { name, .. }) => {
                        let name_str = PyStr::from(name.local_name).into_ref(&vm.ctx);
                        invoke_handler(vm, &self.end_element, (name_str,));
                    }
                    Ok(XmlEvent::Characters(chars)) => {
                        let str = PyStr::from(chars).into_ref(&vm.ctx);
                        invoke_handler(vm, &self.character_data, (str,));
                    }
                    _ => {}
                }
            }
        }

        #[pymethod(name = "Parse")]
        fn parse(&self, data: PyStrRef, _isfinal: OptionalArg<bool>, vm: &VirtualMachine) {
            let reader = Cursor::<Vec<u8>>::new(data.as_bytes().to_vec());
            let parser = self.create_config().create_reader(reader);
            self.do_parse(vm, parser);
        }

        #[pymethod(name = "ParseFile")]
        fn parse_file(&self, file: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // todo: read chunks at a time
            let read_res = vm.call_method(&file, "read", ())?;
            let bytes_like = ArgBytesLike::try_from_object(vm, read_res)?;
            let buf = bytes_like.borrow_buf().to_vec();
            let reader = Cursor::new(buf);
            let parser = self.create_config().create_reader(reader);
            self.do_parse(vm, parser);

            // todo: return value
            Ok(())
        }
    }

    #[derive(FromArgs)]
    #[allow(dead_code)]
    struct ParserCreateArgs {
        #[pyarg(any, optional)]
        encoding: OptionalArg<PyStrRef>,
        #[pyarg(any, optional)]
        namespace_separator: OptionalArg<PyStrRef>,
        #[pyarg(any, optional)]
        intern: OptionalArg<PyStrRef>,
    }

    #[pyfunction(name = "ParserCreate")]
    fn parser_create(
        _args: ParserCreateArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyExpatLikeXmlParserRef> {
        PyExpatLikeXmlParser::new(vm)
    }
}

#[pymodule(name = "model")]
mod _model {}

#[pymodule(name = "errors")]
mod _errors {}
