use super::*;
use rustpython_compiler_core::SourceFile;

// product
impl Node for ast::MatchCase {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            pattern,
            guard,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeMatchCase::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("pattern", pattern.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("guard", guard.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            pattern: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "pattern", "match_case")?,
            )?,
            guard: get_node_field_opt(_vm, &_object, "guard")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            body: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "body", "match_case")?,
            )?,
            range: Default::default(),
        })
    }
}

// sum
impl Node for ast::Pattern {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::MatchValue(cons) => cons.ast_to_object(vm, source_file),
            Self::MatchSingleton(cons) => cons.ast_to_object(vm, source_file),
            Self::MatchSequence(cons) => cons.ast_to_object(vm, source_file),
            Self::MatchMapping(cons) => cons.ast_to_object(vm, source_file),
            Self::MatchClass(cons) => cons.ast_to_object(vm, source_file),
            Self::MatchStar(cons) => cons.ast_to_object(vm, source_file),
            Self::MatchAs(cons) => cons.ast_to_object(vm, source_file),
            Self::MatchOr(cons) => cons.ast_to_object(vm, source_file),
        }
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(pyast::NodePatternMatchValue::static_type()) {
            Self::MatchValue(ast::PatternMatchValue::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodePatternMatchSingleton::static_type()) {
            Self::MatchSingleton(ast::PatternMatchSingleton::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodePatternMatchSequence::static_type()) {
            Self::MatchSequence(ast::PatternMatchSequence::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodePatternMatchMapping::static_type()) {
            Self::MatchMapping(ast::PatternMatchMapping::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodePatternMatchClass::static_type()) {
            Self::MatchClass(ast::PatternMatchClass::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodePatternMatchStar::static_type()) {
            Self::MatchStar(ast::PatternMatchStar::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodePatternMatchAs::static_type()) {
            Self::MatchAs(ast::PatternMatchAs::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodePatternMatchOr::static_type()) {
            Self::MatchOr(ast::PatternMatchOr::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of pattern, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ast::PatternMatchValue {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodePatternMatchValue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            value: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "value", "MatchValue")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "MatchValue")?,
        })
    }
}

// constructor
impl Node for ast::PatternMatchSingleton {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                pyast::NodePatternMatchSingleton::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            value: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "value", "MatchSingleton")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "MatchSingleton")?,
        })
    }
}

impl Node for ast::Singleton {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        match self {
            ast::Singleton::None => vm.ctx.none(),
            ast::Singleton::True => vm.ctx.new_bool(true).into(),
            ast::Singleton::False => vm.ctx.new_bool(false).into(),
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if vm.is_none(&object) {
            Ok(ast::Singleton::None)
        } else if object.is(&vm.ctx.true_value) {
            Ok(ast::Singleton::True)
        } else if object.is(&vm.ctx.false_value) {
            Ok(ast::Singleton::False)
        } else {
            Err(vm.new_value_error(format!(
                "Expected None, True, or False, got {:?}",
                object.class().name()
            )))
        }
    }
}

// constructor
impl Node for ast::PatternMatchSequence {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            patterns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                pyast::NodePatternMatchSequence::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            patterns: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "patterns", "MatchSequence")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "MatchSequence")?,
        })
    }
}

// constructor
impl Node for ast::PatternMatchMapping {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            keys,
            patterns,
            rest,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                pyast::NodePatternMatchMapping::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("keys", keys.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("rest", rest.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            keys: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "keys", "MatchMapping")?,
            )?,
            patterns: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "patterns", "MatchMapping")?,
            )?,
            rest: get_node_field_opt(_vm, &_object, "rest")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            range: range_from_object(_vm, source_file, _object, "MatchMapping")?,
        })
    }
}

// constructor
impl Node for ast::PatternMatchClass {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            cls,
            arguments,
            range: _range,
        } = self;
        let (patterns, kwd_attrs, kwd_patterns) = split_pattern_match_class(arguments);
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodePatternMatchClass::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("cls", cls.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("patterns", patterns.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("kwd_attrs", kwd_attrs.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "kwd_patterns",
            kwd_patterns.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let patterns = Node::ast_from_object(
            vm,
            source_file,
            get_node_field(vm, &object, "patterns", "MatchClass")?,
        )?;
        let kwd_attrs = Node::ast_from_object(
            vm,
            source_file,
            get_node_field(vm, &object, "kwd_attrs", "MatchClass")?,
        )?;
        let kwd_patterns = Node::ast_from_object(
            vm,
            source_file,
            get_node_field(vm, &object, "kwd_patterns", "MatchClass")?,
        )?;
        let (patterns, keywords) = merge_pattern_match_class(patterns, kwd_attrs, kwd_patterns);

        Ok(Self {
            node_index: Default::default(),
            cls: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "cls", "MatchClass")?,
            )?,
            range: range_from_object(vm, source_file, object, "MatchClass")?,
            arguments: ast::PatternArguments {
                node_index: Default::default(),
                range: Default::default(),
                patterns,
                keywords,
            },
        })
    }
}

struct PatternMatchClassPatterns(Vec<ast::Pattern>);

impl Node for PatternMatchClassPatterns {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        self.0.ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self(Node::ast_from_object(vm, source_file, object)?))
    }
}

struct PatternMatchClassKeywordAttributes(Vec<ast::Identifier>);

impl Node for PatternMatchClassKeywordAttributes {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        self.0.ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self(Node::ast_from_object(vm, source_file, object)?))
    }
}

struct PatternMatchClassKeywordPatterns(Vec<ast::Pattern>);

impl Node for PatternMatchClassKeywordPatterns {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        self.0.ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self(Node::ast_from_object(vm, source_file, object)?))
    }
}
// constructor
impl Node for ast::PatternMatchStar {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodePatternMatchStar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            range: range_from_object(_vm, source_file, _object, "MatchStar")?,
        })
    }
}

// constructor
impl Node for ast::PatternMatchAs {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            pattern,
            name,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodePatternMatchAs::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("pattern", pattern.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            pattern: get_node_field_opt(_vm, &_object, "pattern")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            range: range_from_object(_vm, source_file, _object, "MatchAs")?,
        })
    }
}

// constructor
impl Node for ast::PatternMatchOr {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            patterns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodePatternMatchOr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            patterns: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "patterns", "MatchOr")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "MatchOr")?,
        })
    }
}

fn split_pattern_match_class(
    arguments: ast::PatternArguments,
) -> (
    PatternMatchClassPatterns,
    PatternMatchClassKeywordAttributes,
    PatternMatchClassKeywordPatterns,
) {
    let patterns = PatternMatchClassPatterns(arguments.patterns);
    let kwd_attrs = PatternMatchClassKeywordAttributes(
        arguments.keywords.iter().map(|k| k.attr.clone()).collect(),
    );
    let kwd_patterns = PatternMatchClassKeywordPatterns(
        arguments.keywords.into_iter().map(|k| k.pattern).collect(),
    );
    (patterns, kwd_attrs, kwd_patterns)
}

/// Merges the pattern match class attributes and patterns, opposite of [`split_pattern_match_class`].
fn merge_pattern_match_class(
    patterns: PatternMatchClassPatterns,
    kwd_attrs: PatternMatchClassKeywordAttributes,
    kwd_patterns: PatternMatchClassKeywordPatterns,
) -> (Vec<ast::Pattern>, Vec<ast::PatternKeyword>) {
    let keywords = kwd_attrs
        .0
        .into_iter()
        .zip(kwd_patterns.0)
        .map(|(attr, pattern)| ast::PatternKeyword {
            range: Default::default(),
            node_index: Default::default(),
            attr,
            pattern,
        })
        .collect();
    (patterns.0, keywords)
}
