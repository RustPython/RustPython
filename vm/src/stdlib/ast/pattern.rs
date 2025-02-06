use super::*;

// product
impl Node for ruff::MatchCase {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            pattern,
            guard,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeMatchCase::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("pattern", pattern.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("guard", guard.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            pattern: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "pattern", "match_case")?,
            )?,
            guard: get_node_field_opt(_vm, &_object, "guard")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "match_case")?)?,
            range: Default::default(),
        })
    }
}
// sum
impl Node for ruff::Pattern {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ruff::Pattern::MatchValue(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchSingleton(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchSequence(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchMapping(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchClass(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchStar(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchAs(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchOr(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodePatternMatchValue::static_type()) {
            ruff::Pattern::MatchValue(ruff::PatternMatchValue::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchSingleton::static_type()) {
            ruff::Pattern::MatchSingleton(ruff::PatternMatchSingleton::ast_from_object(
                _vm, _object,
            )?)
        } else if _cls.is(gen::NodePatternMatchSequence::static_type()) {
            ruff::Pattern::MatchSequence(ruff::PatternMatchSequence::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchMapping::static_type()) {
            ruff::Pattern::MatchMapping(ruff::PatternMatchMapping::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchClass::static_type()) {
            ruff::Pattern::MatchClass(ruff::PatternMatchClass::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchStar::static_type()) {
            ruff::Pattern::MatchStar(ruff::PatternMatchStar::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchAs::static_type()) {
            ruff::Pattern::MatchAs(ruff::PatternMatchAs::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchOr::static_type()) {
            ruff::Pattern::MatchOr(ruff::PatternMatchOr::ast_from_object(_vm, _object)?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of pattern, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::PatternMatchValue {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchValue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "MatchValue")?,
            )?,
            range: range_from_object(_vm, _object, "MatchValue")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchSingleton {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                gen::NodePatternMatchSingleton::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "MatchSingleton")?,
            )?,
            range: range_from_object(_vm, _object, "MatchSingleton")?,
        })
    }
}
impl Node for ruff::Singleton {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}
// constructor
impl Node for ruff::PatternMatchSequence {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            patterns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchSequence::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            patterns: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "patterns", "MatchSequence")?,
            )?,
            range: range_from_object(_vm, _object, "MatchSequence")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchMapping {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            keys,
            patterns,
            rest,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchMapping::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("keys", keys.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("rest", rest.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            keys: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "keys", "MatchMapping")?,
            )?,
            patterns: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "patterns", "MatchMapping")?,
            )?,
            rest: get_node_field_opt(_vm, &_object, "rest")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "MatchMapping")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchClass {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            cls,
            arguments,
            range: _range,
        } = self;
        let (patterns, kwd_attrs, kwd_patterns) = split_pattern_match_class(arguments);
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodePatternMatchClass::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("cls", cls.ast_to_object(vm), vm).unwrap();
        dict.set_item("patterns", patterns.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("kwd_attrs", kwd_attrs.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("kwd_patterns", kwd_patterns.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, _range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let patterns =
            Node::ast_from_object(vm, get_node_field(vm, &object, "patterns", "MatchClass")?)?;
        let kwd_attrs =
            Node::ast_from_object(vm, get_node_field(vm, &object, "kwd_attrs", "MatchClass")?)?;
        let kwd_patterns = Node::ast_from_object(
            vm,
            get_node_field(vm, &object, "kwd_patterns", "MatchClass")?,
        )?;
        let (patterns, keywords) = merge_pattern_match_class(patterns, kwd_attrs, kwd_patterns);

        Ok(Self {
            cls: Node::ast_from_object(vm, get_node_field(vm, &object, "cls", "MatchClass")?)?,
            range: range_from_object(vm, object, "MatchClass")?,
            arguments: ruff::PatternArguments {
                range: Default::default(),
                patterns,
                keywords,
            },
        })
    }
}

struct PatternMatchClassPatterns {
    pub _range: TextRange, // TODO: Use this
}

impl Node for PatternMatchClassPatterns {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

struct PatternMatchClassKeywordAttributes {
    pub _range: TextRange, // TODO: Use this
}

impl Node for PatternMatchClassKeywordAttributes {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

struct PatternMatchClassKeywordPatterns {
    pub _range: TextRange, // TODO: Use this
}

impl Node for PatternMatchClassKeywordPatterns {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}
// constructor
impl Node for ruff::PatternMatchStar {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchStar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "MatchStar")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchAs {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            pattern,
            name,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchAs::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("pattern", pattern.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            pattern: get_node_field_opt(_vm, &_object, "pattern")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "MatchAs")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchOr {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            patterns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchOr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            patterns: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "patterns", "MatchOr")?,
            )?,
            range: range_from_object(_vm, _object, "MatchOr")?,
        })
    }
}

fn split_pattern_match_class(
    _arguments: ruff::PatternArguments,
) -> (
    PatternMatchClassPatterns,
    PatternMatchClassKeywordAttributes,
    PatternMatchClassKeywordPatterns,
) {
    todo!()
}

/// Merges the pattern match class attributes and patterns, opposite of [`split_pattern_match_class`].
fn merge_pattern_match_class(
    _patterns: PatternMatchClassPatterns,
    _kwd_attrs: PatternMatchClassKeywordAttributes,
    _kwd_patterns: PatternMatchClassKeywordPatterns,
) -> (Vec<ruff::Pattern>, Vec<ruff::PatternKeyword>) {
    todo!()
}
