use super::*;
use rustpython_compiler_core::SourceFile;

// product
impl Node for ast::MatchCase {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index,
            pattern,
            guard,
            body,
            range: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeMatchCase::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("pattern", pattern.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("guard", guard.ast_to_object(vm, source_file), vm)
            .unwrap();
        let body = super::constant::public_ast_stmt_list_object(
            node_index.load(),
            super::constant::PublicAstStmtListField::Body,
        )
        .map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let body: Vec<Option<ast::Stmt>> =
            get_node_list_field(vm, source_file, &object, "body", "match_case")?;
        let (node_index, body) =
            public_stmt_list_from_values(super::constant::PublicAstStmtListField::Body, body);
        Ok(Self {
            node_index,
            pattern: get_required_node_field(vm, source_file, &object, "pattern", "match_case")?,
            guard: get_node_field_opt(vm, &object, "guard")?
                .map(|obj| Node::ast_from_object(vm, source_file, obj))
                .transpose()?,
            body,
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
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if vm.is_none(&object) {
            return Err(vm.new_type_error(format!(
                "expected some sort of pattern, but got {}",
                object.repr(vm)?
            )));
        }
        enum PatternKind {
            MatchValue,
            MatchSingleton,
            MatchSequence,
            MatchMapping,
            MatchClass,
            MatchStar,
            MatchAs,
            MatchOr,
        }
        let kind = if is_node_instance(vm, &object, pyast::NodePatternMatchValue::static_type())? {
            PatternKind::MatchValue
        } else if is_node_instance(vm, &object, pyast::NodePatternMatchSingleton::static_type())? {
            PatternKind::MatchSingleton
        } else if is_node_instance(vm, &object, pyast::NodePatternMatchSequence::static_type())? {
            PatternKind::MatchSequence
        } else if is_node_instance(vm, &object, pyast::NodePatternMatchMapping::static_type())? {
            PatternKind::MatchMapping
        } else if is_node_instance(vm, &object, pyast::NodePatternMatchClass::static_type())? {
            PatternKind::MatchClass
        } else if is_node_instance(vm, &object, pyast::NodePatternMatchStar::static_type())? {
            PatternKind::MatchStar
        } else if is_node_instance(vm, &object, pyast::NodePatternMatchAs::static_type())? {
            PatternKind::MatchAs
        } else if is_node_instance(vm, &object, pyast::NodePatternMatchOr::static_type())? {
            PatternKind::MatchOr
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of pattern, but got {}",
                object.repr(vm)?
            )));
        };
        let range = pattern_range_from_object(vm, source_file, object.clone())?;
        Ok(match kind {
            PatternKind::MatchValue => Self::MatchValue(
                pattern_match_value_from_object_with_range(vm, source_file, object, range)?,
            ),
            PatternKind::MatchSingleton => Self::MatchSingleton(
                pattern_match_singleton_from_object_with_range(vm, source_file, object, range)?,
            ),
            PatternKind::MatchSequence => Self::MatchSequence(
                pattern_match_sequence_from_object_with_range(vm, source_file, object, range)?,
            ),
            PatternKind::MatchMapping => Self::MatchMapping(
                pattern_match_mapping_from_object_with_range(vm, source_file, object, range)?,
            ),
            PatternKind::MatchClass => Self::MatchClass(
                pattern_match_class_from_object_with_range(vm, source_file, object, range)?,
            ),
            PatternKind::MatchStar => Self::MatchStar(pattern_match_star_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            PatternKind::MatchAs => Self::MatchAs(pattern_match_as_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            PatternKind::MatchOr => Self::MatchOr(pattern_match_or_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
        })
    }
}

fn pattern_node_index(index: ast::NodeIndex) -> ast::AtomicNodeIndex {
    let node_index = ast::AtomicNodeIndex::NONE;
    node_index.set(index);
    node_index
}

fn null_pattern_placeholder(range: TextRange) -> ast::Pattern {
    ast::Pattern::MatchAs(ast::PatternMatchAs {
        node_index: Default::default(),
        range,
        pattern: None,
        name: None,
    })
}

fn lower_nullable_patterns(values: &[Option<ast::Pattern>], range: TextRange) -> ast::Patterns {
    values
        .iter()
        .cloned()
        .map(|value| value.unwrap_or_else(|| null_pattern_placeholder(range)))
        .collect()
}

fn null_expr_placeholder(range: TextRange) -> ast::Expr {
    ast::Expr::NoneLiteral(ast::ExprNoneLiteral {
        node_index: Default::default(),
        range,
    })
}

fn lower_nullable_exprs(values: &[Option<ast::Expr>], range: TextRange) -> ast::PatternKeys {
    values
        .iter()
        .cloned()
        .map(|value| value.unwrap_or_else(|| null_expr_placeholder(range)))
        .collect()
}

fn pattern_list_from_field(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: &PyObject,
    field: &'static str,
    typ: &str,
    range: TextRange,
) -> PyResult<(ast::AtomicNodeIndex, ast::Patterns)> {
    let values: Vec<Option<ast::Pattern>> =
        get_node_list_field(vm, source_file, object, field, typ)?;
    let node_index = if values.iter().any(Option::is_none) {
        pattern_node_index(super::constant::register_public_ast_pattern_list(
            values.clone(),
        ))
    } else {
        Default::default()
    };
    Ok((node_index, lower_nullable_patterns(&values, range)))
}

// constructor
fn pattern_match_value_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::PatternMatchValue> {
    Ok(ast::PatternMatchValue {
        node_index: Default::default(),
        value: get_required_node_field(vm, source_file, &object, "value", "MatchValue")?,
        range,
    })
}

impl Node for ast::PatternMatchValue {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodePatternMatchValue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "MatchValue")?;
        pattern_match_value_from_object_with_range(vm, source_file, object, range)
    }
}

// constructor
fn pattern_match_singleton_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::PatternMatchSingleton> {
    Ok(ast::PatternMatchSingleton {
        node_index: Default::default(),
        value: Node::ast_from_object(
            vm,
            source_file,
            get_node_field(vm, &object, "value", "MatchSingleton")?,
        )?,
        range,
    })
}

impl Node for ast::PatternMatchSingleton {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                vm,
                pyast::NodePatternMatchSingleton::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "MatchSingleton")?;
        pattern_match_singleton_from_object_with_range(vm, source_file, object, range)
    }
}

impl Node for ast::Singleton {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::None => vm.ctx.none(),
            Self::True => vm.ctx.new_bool(true).into(),
            Self::False => vm.ctx.new_bool(false).into(),
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if vm.is_none(&object) {
            Ok(Self::None)
        } else if object.is(&vm.ctx.true_value) {
            Ok(Self::True)
        } else if object.is(&vm.ctx.false_value) {
            Ok(Self::False)
        } else {
            Err(vm.new_value_error("MatchSingleton can only contain True, False and None"))
        }
    }
}

// constructor
fn pattern_match_sequence_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::PatternMatchSequence> {
    let (node_index, patterns) =
        pattern_list_from_field(vm, source_file, &object, "patterns", "MatchSequence", range)?;
    Ok(ast::PatternMatchSequence {
        node_index,
        patterns: patterns.to_vec(),
        range,
    })
}

impl Node for ast::PatternMatchSequence {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index,
            patterns,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                vm,
                pyast::NodePatternMatchSequence::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let patterns = super::constant::public_ast_pattern_list_object(node_index.load())
            .map_or_else(
                || patterns.ast_to_object(vm, source_file),
                |values| values.values.ast_to_object(vm, source_file),
            );
        dict.set_item("patterns", patterns, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "MatchSequence")?;
        pattern_match_sequence_from_object_with_range(vm, source_file, object, range)
    }
}

// constructor
fn pattern_match_mapping_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::PatternMatchMapping> {
    let keys: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, &object, "keys", "MatchMapping")?;
    let patterns: Vec<Option<ast::Pattern>> =
        get_node_list_field(vm, source_file, &object, "patterns", "MatchMapping")?;
    let has_public_override =
        keys.iter().any(Option::is_none) || patterns.iter().any(Option::is_none);
    let node_index = if has_public_override {
        pattern_node_index(super::constant::register_public_ast_match_mapping(
            keys.clone(),
            patterns.clone(),
        ))
    } else {
        Default::default()
    };
    Ok(ast::PatternMatchMapping {
        node_index,
        keys: lower_nullable_exprs(&keys, range),
        patterns: lower_nullable_patterns(&patterns, range),
        rest: get_node_field_opt(vm, &object, "rest")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::PatternMatchMapping {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index,
            keys,
            patterns,
            rest,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodePatternMatchMapping::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let keys = super::constant::public_ast_expr_option_list_object(node_index.load())
            .map_or_else(
                || keys.ast_to_object(vm, source_file),
                |values| values.values.ast_to_object(vm, source_file),
            );
        dict.set_item("keys", keys, vm).unwrap();
        let patterns = super::constant::public_ast_pattern_list_object(node_index.load())
            .map_or_else(
                || patterns.ast_to_object(vm, source_file),
                |values| values.values.ast_to_object(vm, source_file),
            );
        dict.set_item("patterns", patterns, vm).unwrap();
        dict.set_item("rest", rest.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "MatchMapping")?;
        pattern_match_mapping_from_object_with_range(vm, source_file, object, range)
    }
}

// constructor
fn pattern_match_class_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::PatternMatchClass> {
    let cls = get_required_node_field(vm, source_file, &object, "cls", "MatchClass")?;
    let patterns: Vec<Option<ast::Pattern>> =
        get_node_list_field(vm, source_file, &object, "patterns", "MatchClass")?;
    let kwd_attrs = PatternMatchClassKeywordAttributes::ast_from_field(
        vm,
        source_file,
        &object,
        "kwd_attrs",
        "MatchClass",
    )?;
    let kwd_patterns: Vec<Option<ast::Pattern>> =
        get_node_list_field(vm, source_file, &object, "kwd_patterns", "MatchClass")?;
    let has_public_override = kwd_attrs.0.len() != kwd_patterns.len()
        || patterns.iter().any(Option::is_none)
        || kwd_patterns.iter().any(Option::is_none);
    let node_index = if has_public_override {
        pattern_node_index(super::constant::register_public_ast_match_class(
            patterns.clone(),
            kwd_attrs.0.clone(),
            kwd_patterns.clone(),
        ))
    } else {
        Default::default()
    };
    let patterns = PatternMatchClassPatterns(lower_nullable_patterns(&patterns, range));
    let kwd_patterns =
        PatternMatchClassKeywordPatterns(lower_nullable_patterns(&kwd_patterns, range));
    let (patterns, keywords) = merge_pattern_match_class(patterns, kwd_attrs, kwd_patterns);

    Ok(ast::PatternMatchClass {
        node_index,
        cls,
        range,
        arguments: ast::PatternArguments {
            node_index: Default::default(),
            range: Default::default(),
            patterns,
            keywords,
        },
    })
}

impl Node for ast::PatternMatchClass {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index,
            cls,
            arguments,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodePatternMatchClass::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("cls", cls.ast_to_object(vm, source_file), vm)
            .unwrap();
        let (patterns, kwd_attrs, kwd_patterns) = if let Some(values) =
            super::constant::public_ast_match_class_object(node_index.load())
        {
            (
                values.patterns.ast_to_object(vm, source_file),
                values.kwd_attrs.ast_to_object(vm, source_file),
                values.kwd_patterns.ast_to_object(vm, source_file),
            )
        } else {
            let (patterns, kwd_attrs, kwd_patterns) = split_pattern_match_class(arguments);
            (
                patterns.ast_to_object(vm, source_file),
                kwd_attrs.ast_to_object(vm, source_file),
                kwd_patterns.ast_to_object(vm, source_file),
            )
        };
        dict.set_item("patterns", patterns, vm).unwrap();
        dict.set_item("kwd_attrs", kwd_attrs, vm).unwrap();
        dict.set_item("kwd_patterns", kwd_patterns, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "MatchClass")?;
        pattern_match_class_from_object_with_range(vm, source_file, object, range)
    }
}

struct PatternMatchClassPatterns(ast::Patterns);

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

impl PatternMatchClassKeywordAttributes {
    fn ast_from_field(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        Ok(Self(get_node_list_field(
            vm,
            source_file,
            object,
            field,
            typ,
        )?))
    }
}

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

struct PatternMatchClassKeywordPatterns(ast::Patterns);

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
fn pattern_match_star_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::PatternMatchStar> {
    Ok(ast::PatternMatchStar {
        node_index: Default::default(),
        name: get_node_field_opt(vm, &object, "name")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::PatternMatchStar {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodePatternMatchStar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "MatchStar")?;
        pattern_match_star_from_object_with_range(vm, source_file, object, range)
    }
}

// constructor
fn pattern_match_as_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::PatternMatchAs> {
    Ok(ast::PatternMatchAs {
        node_index: Default::default(),
        pattern: get_node_field_opt(vm, &object, "pattern")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        name: get_node_field_opt(vm, &object, "name")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::PatternMatchAs {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            pattern,
            name,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodePatternMatchAs::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("pattern", pattern.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "MatchAs")?;
        pattern_match_as_from_object_with_range(vm, source_file, object, range)
    }
}

// constructor
fn pattern_match_or_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::PatternMatchOr> {
    let (node_index, patterns) =
        pattern_list_from_field(vm, source_file, &object, "patterns", "MatchOr", range)?;
    Ok(ast::PatternMatchOr {
        node_index,
        patterns: patterns.to_vec(),
        range,
    })
}

impl Node for ast::PatternMatchOr {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index,
            patterns,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodePatternMatchOr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let patterns = super::constant::public_ast_pattern_list_object(node_index.load())
            .map_or_else(
                || patterns.ast_to_object(vm, source_file),
                |values| values.values.ast_to_object(vm, source_file),
            );
        dict.set_item("patterns", patterns, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "MatchOr")?;
        pattern_match_or_from_object_with_range(vm, source_file, object, range)
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
) -> (ast::Patterns, Vec<ast::PatternKeyword>) {
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
