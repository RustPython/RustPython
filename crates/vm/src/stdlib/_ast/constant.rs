use super::*;
use crate::builtins::{PyComplex, PyFrozenSet, PyTuple};
use ast::str_prefix::StringLiteralPrefix;
use core::cell::RefCell;
use rustpython_codegen::{
    PublicAstExprList, PublicAstFormattedValue, PublicAstInterpolation, PublicAstNodeMap,
    compile::ruff_int_to_bigint,
};
use rustpython_compiler_core::{SourceFile, bytecode::ConstantData};

#[derive(Clone)]
pub(super) struct PublicAstPatternList {
    pub(super) values: Vec<Option<ast::Pattern>>,
}

#[derive(Clone)]
pub(super) struct PublicAstExprOptionList {
    pub(super) values: Vec<Option<ast::Expr>>,
}

#[derive(Clone)]
pub(super) struct PublicAstStmtList {
    pub(super) values: Vec<Option<ast::Stmt>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum PublicAstStmtListField {
    Body,
    Orelse,
    FinalBody,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum PublicAstExprListField {
    Args,
    Bases,
    DecoratorList,
    Targets,
    Values,
    Elts,
    Comparators,
    Ifs,
}

#[derive(Clone)]
pub(super) struct PublicAstExceptHandlerList {
    pub(super) values: Vec<Option<ast::ExceptHandler>>,
}

#[derive(Clone)]
pub(super) struct PublicAstTypeParamList {
    pub(super) values: Vec<Option<ast::TypeParam>>,
}

#[derive(Clone)]
pub(super) struct PublicAstMatchClass {
    pub(super) patterns: Vec<Option<ast::Pattern>>,
    pub(super) kwd_attrs: Vec<ast::Identifier>,
    pub(super) kwd_patterns: Vec<Option<ast::Pattern>>,
}

#[derive(Clone, Default)]
pub(super) struct PublicAstExprListFields {
    args: Option<PublicAstExprOptionList>,
    bases: Option<PublicAstExprOptionList>,
    decorator_list: Option<PublicAstExprOptionList>,
    targets: Option<PublicAstExprOptionList>,
    values: Option<PublicAstExprOptionList>,
    elts: Option<PublicAstExprOptionList>,
    comparators: Option<PublicAstExprOptionList>,
    ifs: Option<PublicAstExprOptionList>,
}

impl PublicAstExprListFields {
    fn insert(&mut self, field: PublicAstExprListField, values: PublicAstExprOptionList) {
        let slot = match field {
            PublicAstExprListField::Args => &mut self.args,
            PublicAstExprListField::Bases => &mut self.bases,
            PublicAstExprListField::DecoratorList => &mut self.decorator_list,
            PublicAstExprListField::Targets => &mut self.targets,
            PublicAstExprListField::Values => &mut self.values,
            PublicAstExprListField::Elts => &mut self.elts,
            PublicAstExprListField::Comparators => &mut self.comparators,
            PublicAstExprListField::Ifs => &mut self.ifs,
        };
        *slot = Some(values);
    }

    pub(super) fn get(&self, field: PublicAstExprListField) -> Option<&PublicAstExprOptionList> {
        match field {
            PublicAstExprListField::Args => self.args.as_ref(),
            PublicAstExprListField::Bases => self.bases.as_ref(),
            PublicAstExprListField::DecoratorList => self.decorator_list.as_ref(),
            PublicAstExprListField::Targets => self.targets.as_ref(),
            PublicAstExprListField::Values => self.values.as_ref(),
            PublicAstExprListField::Elts => self.elts.as_ref(),
            PublicAstExprListField::Comparators => self.comparators.as_ref(),
            PublicAstExprListField::Ifs => self.ifs.as_ref(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct PublicAstStmtListFields {
    body: Option<PublicAstStmtList>,
    orelse: Option<PublicAstStmtList>,
    finalbody: Option<PublicAstStmtList>,
}

impl PublicAstStmtListFields {
    fn insert(&mut self, field: PublicAstStmtListField, values: PublicAstStmtList) {
        let slot = match field {
            PublicAstStmtListField::Body => &mut self.body,
            PublicAstStmtListField::Orelse => &mut self.orelse,
            PublicAstStmtListField::FinalBody => &mut self.finalbody,
        };
        *slot = Some(values);
    }

    pub(super) fn get(&self, field: PublicAstStmtListField) -> Option<&PublicAstStmtList> {
        match field {
            PublicAstStmtListField::Body => self.body.as_ref(),
            PublicAstStmtListField::Orelse => self.orelse.as_ref(),
            PublicAstStmtListField::FinalBody => self.finalbody.as_ref(),
        }
    }
}

#[derive(Debug)]
pub(super) struct Constant {
    pub(super) range: TextRange,
    pub(super) value: ConstantLiteral,
    invalid_type: Option<String>,
}

impl Constant {
    pub(super) fn new_str(
        value: impl Into<Box<str>>,
        prefix: StringLiteralPrefix,
        range: TextRange,
    ) -> Self {
        let value = value.into();
        Self {
            range,
            value: ConstantLiteral::Str { value, prefix },
            invalid_type: None,
        }
    }

    pub(super) const fn new_int(value: ast::Int, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Int(value),
            invalid_type: None,
        }
    }

    pub(super) const fn new_float(value: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Float(value),
            invalid_type: None,
        }
    }

    pub(super) const fn new_complex(real: f64, imag: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Complex { real, imag },
            invalid_type: None,
        }
    }

    pub(super) const fn new_bytes(value: Box<[u8]>, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bytes(value),
            invalid_type: None,
        }
    }

    pub(super) const fn new_bool(value: bool, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bool(value),
            invalid_type: None,
        }
    }

    pub(super) const fn new_none(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::None,
            invalid_type: None,
        }
    }

    pub(super) const fn new_ellipsis(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Ellipsis,
            invalid_type: None,
        }
    }

    pub(crate) fn into_expr(self) -> ast::Expr {
        let invalid_type = self.invalid_type.clone();
        let constant = self
            .invalid_type
            .is_none()
            .then(|| constant_literal_to_constant_data(&self.value));
        let expr = constant_to_ruff_expr(self);
        if let Some(invalid_type) = invalid_type {
            register_public_ast_invalid_constant(&expr, invalid_type);
        } else if let Some(constant) = constant {
            register_public_ast_constant(&expr, constant);
        }
        expr
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ConstantLiteral {
    None,
    Bool(bool),
    Str {
        value: Box<str>,
        prefix: StringLiteralPrefix,
    },
    Bytes(Box<[u8]>),
    Int(ast::Int),
    Tuple(Vec<Self>),
    FrozenSet(Vec<Self>),
    Float(f64),
    Complex {
        real: f64,
        imag: f64,
    },
    Ellipsis,
}

struct PublicAstConstantState {
    next_index: u32,
    // CPython AST has Constant_kind.value/kind fields; Ruff has separate
    // literal expr variants. Dense node indexes make Vec lookup cheaper than
    // hashing, and insertion order is never observed.
    constants: PublicAstNodeMap<ConstantData>,
    // CPython Interpolation has raw str and expr? format_spec; Ruff t-string
    // elements do not. Dense node lookup avoids hashing these synthetic nodes.
    interpolations: PublicAstNodeMap<PublicAstInterpolation>,
    // CPython FormattedValue has expr? format_spec; Ruff f-string specs are
    // parsed as string elements. Dense node lookup is the direct hot path.
    formatted_values: PublicAstNodeMap<PublicAstFormattedValue>,
    // CPython ImportFrom.level accepts a public signed int; Ruff only stores
    // parser-valid unsigned levels. Dense lookup preserves only overrides.
    import_from_levels: PublicAstNodeMap<i32>,
    // CPython validates Constant.value after object conversion; Ruff has no
    // invalid Constant node. Dense lookup stores only rejected public values.
    invalid_constants: PublicAstNodeMap<String>,
    // CPython JoinedStr.values is expr*; Ruff stores f-string element trees.
    // Dense lookup restores the public expr list without ordered-map overhead.
    joined_strs: PublicAstNodeMap<PublicAstExprList>,
    // CPython TemplateStr.values is expr*; Ruff stores t-string element trees.
    // Dense lookup restores the public expr list without ordered-map overhead.
    template_strs: PublicAstNodeMap<PublicAstExprList>,
    // CPython comprehension has is_async; Ruff folds it into generator data.
    // Dense lookup keeps the raw public flag on affected nodes only.
    comprehension_is_async: PublicAstNodeMap<i32>,
    // CPython permits nullable public pattern lists during validation; Ruff
    // pattern lists are non-null. Dense lookup stores only nullable lists.
    pattern_lists: PublicAstNodeMap<PublicAstPatternList>,
    // CPython has nullable expr?* slots such as defaults; Ruff omits null list
    // entries. Dense lookup stores only public nullable-list nodes.
    expr_option_lists: PublicAstNodeMap<PublicAstExprOptionList>,
    // CPython public expr* fields may contain None until validation; Ruff
    // Vec<Expr> cannot represent null entries. Per-node bundles avoid hashing.
    expr_lists: PublicAstNodeMap<PublicAstExprListFields>,
    // CPython public stmt* fields may contain None until validation; Ruff
    // Vec<Stmt> cannot represent null entries. Per-node bundles avoid hashing.
    stmt_lists: PublicAstNodeMap<PublicAstStmtListFields>,
    // CPython nullable excepthandler* lists cannot be represented in Ruff.
    // Dense lookup stores only public nodes that need nullable validation.
    except_handler_lists: PublicAstNodeMap<PublicAstExceptHandlerList>,
    // CPython nullable type_param* lists cannot be represented in Ruff. Dense
    // lookup stores only public nodes that need nullable validation.
    type_param_lists: PublicAstNodeMap<PublicAstTypeParamList>,
    // CPython MatchClass splits patterns/kwd_attrs/kwd_patterns; Ruff stores
    // PatternArguments. Dense lookup restores the public split shape.
    match_classes: PublicAstNodeMap<PublicAstMatchClass>,
    // CPython AnnAssign.simple is a raw int; Ruff has no equivalent field.
    // Dense lookup stores only public AnnAssign overrides.
    ann_assign_simple: PublicAstNodeMap<i32>,
    // CPython arg nodes have type_comment; Ruff parameters do not. Dense lookup
    // stores only public arg comments.
    arg_type_comments: PublicAstNodeMap<PyObjectRef>,
    // CPython selected stmt nodes have type_comment; Ruff omits them. Dense
    // lookup stores only public stmt comments.
    stmt_type_comments: PublicAstNodeMap<PyObjectRef>,
}

type PublicAstOverrideMap = PublicAstNodeMap<ConstantData>;
type PublicAstInterpolationOverrideMap = PublicAstNodeMap<PublicAstInterpolation>;
type PublicAstFormattedValueOverrideMap = PublicAstNodeMap<PublicAstFormattedValue>;
pub(super) type PublicAstImportFromLevelOverrideMap = PublicAstNodeMap<i32>;
pub(super) type PublicAstInvalidConstantOverrideMap = PublicAstNodeMap<String>;
pub(super) type PublicAstExprListOverrideMap = PublicAstNodeMap<PublicAstExprList>;
pub(super) type PublicAstComprehensionIsAsyncOverrideMap = PublicAstNodeMap<i32>;
pub(super) type PublicAstPatternListOverrideMap = PublicAstNodeMap<PublicAstPatternList>;
pub(super) type PublicAstExprOptionListOverrideMap = PublicAstNodeMap<PublicAstExprOptionList>;
pub(super) type PublicAstExprListFieldOverrideMap = PublicAstNodeMap<PublicAstExprListFields>;
pub(super) type PublicAstStmtListOverrideMap = PublicAstNodeMap<PublicAstStmtListFields>;
pub(super) type PublicAstExceptHandlerListOverrideMap =
    PublicAstNodeMap<PublicAstExceptHandlerList>;
pub(super) type PublicAstTypeParamListOverrideMap = PublicAstNodeMap<PublicAstTypeParamList>;
pub(super) type PublicAstMatchClassOverrideMap = PublicAstNodeMap<PublicAstMatchClass>;
pub(super) type PublicAstAnnAssignSimpleOverrideMap = PublicAstNodeMap<i32>;
pub(super) type PublicAstArgTypeCommentOverrideMap = PublicAstNodeMap<PyObjectRef>;
pub(super) type PublicAstStmtTypeCommentOverrideMap = PublicAstNodeMap<PyObjectRef>;
type PublicAstOverrideCollection<T> = (
    T,
    PublicAstOverrideMap,
    PublicAstInterpolationOverrideMap,
    PublicAstFormattedValueOverrideMap,
    PublicAstImportFromLevelOverrideMap,
    PublicAstInvalidConstantOverrideMap,
    PublicAstExprListOverrideMap,
    PublicAstExprListOverrideMap,
    PublicAstComprehensionIsAsyncOverrideMap,
    PublicAstPatternListOverrideMap,
    PublicAstExprOptionListOverrideMap,
    PublicAstExprListFieldOverrideMap,
    PublicAstStmtListOverrideMap,
    PublicAstExceptHandlerListOverrideMap,
    PublicAstTypeParamListOverrideMap,
    PublicAstMatchClassOverrideMap,
    PublicAstAnnAssignSimpleOverrideMap,
    PublicAstArgTypeCommentOverrideMap,
    PublicAstStmtTypeCommentOverrideMap,
);

thread_local! {
    static PUBLIC_AST_CONSTANTS: RefCell<Option<PublicAstConstantState>> = const { RefCell::new(None) };
    static PUBLIC_AST_CONSTANT_OBJECTS: RefCell<Option<PublicAstOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_INTERPOLATION_OBJECTS: RefCell<Option<PublicAstInterpolationOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_FORMATTED_VALUE_OBJECTS: RefCell<Option<PublicAstFormattedValueOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_JOINED_STR_OBJECTS: RefCell<Option<PublicAstExprListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_TEMPLATE_STR_OBJECTS: RefCell<Option<PublicAstExprListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_COMPREHENSION_IS_ASYNC_OBJECTS: RefCell<Option<PublicAstComprehensionIsAsyncOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_PATTERN_LIST_OBJECTS: RefCell<Option<PublicAstPatternListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_EXPR_OPTION_LIST_OBJECTS: RefCell<Option<PublicAstExprOptionListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_EXPR_LIST_OBJECTS: RefCell<Option<PublicAstExprListFieldOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_STMT_LIST_OBJECTS: RefCell<Option<PublicAstStmtListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_EXCEPT_HANDLER_LIST_OBJECTS: RefCell<Option<PublicAstExceptHandlerListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_TYPE_PARAM_LIST_OBJECTS: RefCell<Option<PublicAstTypeParamListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_MATCH_CLASS_OBJECTS: RefCell<Option<PublicAstMatchClassOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_ANN_ASSIGN_SIMPLE_OBJECTS: RefCell<Option<PublicAstAnnAssignSimpleOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_ARG_TYPE_COMMENT_OBJECTS: RefCell<Option<PublicAstArgTypeCommentOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_STMT_TYPE_COMMENT_OBJECTS: RefCell<Option<PublicAstStmtTypeCommentOverrideMap>> = const { RefCell::new(None) };
}

pub(super) fn collect_public_ast_overrides<T>(
    f: impl FnOnce() -> PyResult<T>,
) -> PyResult<PublicAstOverrideCollection<T>> {
    PUBLIC_AST_CONSTANTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(PublicAstConstantState {
            next_index: 0,
            constants: PublicAstNodeMap::new(),
            interpolations: PublicAstNodeMap::new(),
            formatted_values: PublicAstNodeMap::new(),
            import_from_levels: PublicAstNodeMap::new(),
            invalid_constants: PublicAstNodeMap::new(),
            joined_strs: PublicAstNodeMap::new(),
            template_strs: PublicAstNodeMap::new(),
            comprehension_is_async: PublicAstNodeMap::new(),
            pattern_lists: PublicAstNodeMap::new(),
            expr_option_lists: PublicAstNodeMap::new(),
            expr_lists: PublicAstNodeMap::new(),
            stmt_lists: PublicAstNodeMap::new(),
            except_handler_lists: PublicAstNodeMap::new(),
            type_param_lists: PublicAstNodeMap::new(),
            match_classes: PublicAstNodeMap::new(),
            ann_assign_simple: PublicAstNodeMap::new(),
            arg_type_comments: PublicAstNodeMap::new(),
            stmt_type_comments: PublicAstNodeMap::new(),
        });
    });

    let result = f();
    let (
        constants,
        interpolations,
        formatted_values,
        import_from_levels,
        invalid_constants,
        joined_strs,
        template_strs,
        comprehension_is_async,
        pattern_lists,
        expr_option_lists,
        expr_lists,
        stmt_lists,
        except_handler_lists,
        type_param_lists,
        match_classes,
        ann_assign_simple,
        arg_type_comments,
        stmt_type_comments,
    ) = PUBLIC_AST_CONSTANTS.with(|cell| {
        let state = cell
            .borrow_mut()
            .take()
            .expect("public AST constant collection state missing");
        (
            state.constants,
            state.interpolations,
            state.formatted_values,
            state.import_from_levels,
            state.invalid_constants,
            state.joined_strs,
            state.template_strs,
            state.comprehension_is_async,
            state.pattern_lists,
            state.expr_option_lists,
            state.expr_lists,
            state.stmt_lists,
            state.except_handler_lists,
            state.type_param_lists,
            state.match_classes,
            state.ann_assign_simple,
            state.arg_type_comments,
            state.stmt_type_comments,
        )
    });
    result.map(|value| {
        (
            value,
            constants,
            interpolations,
            formatted_values,
            import_from_levels,
            invalid_constants,
            joined_strs,
            template_strs,
            comprehension_is_async,
            pattern_lists,
            expr_option_lists,
            expr_lists,
            stmt_lists,
            except_handler_lists,
            type_param_lists,
            match_classes,
            ann_assign_simple,
            arg_type_comments,
            stmt_type_comments,
        )
    })
}

fn register_public_ast_constant(expr: &ast::Expr, constant: ConstantData) {
    let index = register_public_ast_override(|state, index| {
        state.constants.insert(index, constant);
    });
    ast::HasNodeIndex::node_index(expr).set(index);
}

fn register_public_ast_invalid_constant(expr: &ast::Expr, invalid_type: String) {
    let index = register_public_ast_override(|state, index| {
        state.invalid_constants.insert(index, invalid_type);
    });
    ast::HasNodeIndex::node_index(expr).set(index);
}

pub(super) fn register_public_ast_interpolation(
    str_constant: ConstantData,
    format_spec: Option<Box<ast::Expr>>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state.interpolations.insert(
            index,
            PublicAstInterpolation {
                str: str_constant,
                format_spec,
            },
        );
    })
}

pub(super) fn register_public_ast_formatted_value(
    format_spec: Option<Box<ast::Expr>>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state
            .formatted_values
            .insert(index, PublicAstFormattedValue { format_spec });
    })
}

pub(super) fn register_public_ast_joined_str(values: Vec<ast::Expr>) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state
            .joined_strs
            .insert(index, PublicAstExprList { values });
    })
}

pub(super) fn register_public_ast_template_str(values: Vec<ast::Expr>) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state
            .template_strs
            .insert(index, PublicAstExprList { values });
    })
}

pub(super) fn register_public_ast_pattern_list(
    values: Vec<Option<ast::Pattern>>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state
            .pattern_lists
            .insert(index, PublicAstPatternList { values });
    })
}

pub(super) fn register_public_ast_match_mapping(
    keys: Vec<Option<ast::Expr>>,
    patterns: Vec<Option<ast::Pattern>>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state
            .expr_option_lists
            .insert(index, PublicAstExprOptionList { values: keys });
        state
            .pattern_lists
            .insert(index, PublicAstPatternList { values: patterns });
    })
}

pub(super) fn register_public_ast_expr_option_list(
    values: Vec<Option<ast::Expr>>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state
            .expr_option_lists
            .insert(index, PublicAstExprOptionList { values });
    })
}

pub(super) fn register_public_ast_stmt_list(
    field: PublicAstStmtListField,
    values: Vec<Option<ast::Stmt>>,
) -> ast::NodeIndex {
    register_public_ast_stmt_lists([(field, values)])
}

pub(super) fn register_public_ast_stmt_lists(
    values: impl IntoIterator<Item = (PublicAstStmtListField, Vec<Option<ast::Stmt>>)>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        for (field, values) in values {
            public_ast_stmt_fields_mut(&mut state.stmt_lists, index)
                .insert(field, PublicAstStmtList { values });
        }
    })
}

pub(super) fn register_public_ast_try_lists(
    stmt_values: Vec<(PublicAstStmtListField, Vec<Option<ast::Stmt>>)>,
    except_handler_values: Option<Vec<Option<ast::ExceptHandler>>>,
) -> ast::NodeIndex {
    register_public_ast_node_list_overrides(stmt_values, Vec::new(), except_handler_values, None)
}

pub(super) fn register_public_ast_node_list_overrides(
    stmt_values: Vec<(PublicAstStmtListField, Vec<Option<ast::Stmt>>)>,
    expr_values: Vec<(PublicAstExprListField, Vec<Option<ast::Expr>>)>,
    except_handler_values: Option<Vec<Option<ast::ExceptHandler>>>,
    comprehension_is_async: Option<i32>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        for (field, values) in stmt_values {
            public_ast_stmt_fields_mut(&mut state.stmt_lists, index)
                .insert(field, PublicAstStmtList { values });
        }
        for (field, values) in expr_values {
            public_ast_expr_fields_mut(&mut state.expr_lists, index)
                .insert(field, PublicAstExprOptionList { values });
        }
        if let Some(values) = except_handler_values {
            state
                .except_handler_lists
                .insert(index, PublicAstExceptHandlerList { values });
        }
        if let Some(value) = comprehension_is_async {
            state.comprehension_is_async.insert(index, value);
        }
    })
}

fn public_ast_expr_fields_mut(
    values: &mut PublicAstExprListFieldOverrideMap,
    index: ast::NodeIndex,
) -> &mut PublicAstExprListFields {
    if !values.contains_key(&index) {
        values.insert(index, PublicAstExprListFields::default());
    }
    values.get_mut(&index).unwrap()
}

fn public_ast_stmt_fields_mut(
    values: &mut PublicAstStmtListOverrideMap,
    index: ast::NodeIndex,
) -> &mut PublicAstStmtListFields {
    if !values.contains_key(&index) {
        values.insert(index, PublicAstStmtListFields::default());
    }
    values.get_mut(&index).unwrap()
}

pub(super) fn register_public_ast_type_param_list(
    values: Vec<Option<ast::TypeParam>>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state
            .type_param_lists
            .insert(index, PublicAstTypeParamList { values });
    })
}

pub(super) fn register_public_ast_match_class(
    patterns: Vec<Option<ast::Pattern>>,
    kwd_attrs: Vec<ast::Identifier>,
    kwd_patterns: Vec<Option<ast::Pattern>>,
) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state.match_classes.insert(
            index,
            PublicAstMatchClass {
                patterns,
                kwd_attrs,
                kwd_patterns,
            },
        );
    })
}

pub(super) fn register_public_ast_import_from_level(level: i32) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state.import_from_levels.insert(index, level);
    })
}

pub(super) fn register_public_ast_ann_assign_simple(simple: i32) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state.ann_assign_simple.insert(index, simple);
    })
}

pub(super) fn register_public_ast_arg_type_comment(type_comment: PyObjectRef) -> ast::NodeIndex {
    register_public_ast_override(|state, index| {
        state.arg_type_comments.insert(index, type_comment);
    })
}

pub(super) fn register_public_ast_stmt_type_comment(
    node_index: &ast::AtomicNodeIndex,
    type_comment: PyObjectRef,
) {
    register_public_ast_node_override(node_index, |state, index| {
        state.stmt_type_comments.insert(index, type_comment);
    });
}

fn register_public_ast_override(
    insert: impl FnOnce(&mut PublicAstConstantState, ast::NodeIndex),
) -> ast::NodeIndex {
    PUBLIC_AST_CONSTANTS.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(state) = state.as_mut() else {
            return ast::NodeIndex::NONE;
        };
        let index = ast::NodeIndex::from(state.next_index);
        state.next_index = state
            .next_index
            .checked_add(1)
            .expect("too many public AST constants");
        insert(state, index);
        index
    })
}

fn register_public_ast_node_override(
    node_index: &ast::AtomicNodeIndex,
    insert: impl FnOnce(&mut PublicAstConstantState, ast::NodeIndex),
) {
    PUBLIC_AST_CONSTANTS.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        let mut index = node_index.load();
        if index == ast::NodeIndex::NONE {
            index = ast::NodeIndex::from(state.next_index);
            state.next_index = state
                .next_index
                .checked_add(1)
                .expect("too many public AST constants");
            node_index.set(index);
        }
        insert(state, index);
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "public AST conversion installs independent override tables"
)]
pub(super) fn with_public_ast_interpolation_objects<T>(
    constants: &PublicAstOverrideMap,
    interpolations: &PublicAstInterpolationOverrideMap,
    formatted_values: &PublicAstFormattedValueOverrideMap,
    joined_strs: &PublicAstExprListOverrideMap,
    template_strs: &PublicAstExprListOverrideMap,
    comprehension_is_async: &PublicAstComprehensionIsAsyncOverrideMap,
    pattern_lists: &PublicAstPatternListOverrideMap,
    expr_option_lists: &PublicAstExprOptionListOverrideMap,
    expr_lists: &PublicAstExprListFieldOverrideMap,
    stmt_lists: &PublicAstStmtListOverrideMap,
    except_handler_lists: &PublicAstExceptHandlerListOverrideMap,
    type_param_lists: &PublicAstTypeParamListOverrideMap,
    match_classes: &PublicAstMatchClassOverrideMap,
    ann_assign_simple: &PublicAstAnnAssignSimpleOverrideMap,
    arg_type_comments: &PublicAstArgTypeCommentOverrideMap,
    stmt_type_comments: &PublicAstStmtTypeCommentOverrideMap,
    f: impl FnOnce() -> T,
) -> T {
    PUBLIC_AST_CONSTANT_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(constants.clone());
    });
    PUBLIC_AST_INTERPOLATION_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(interpolations.clone());
    });
    PUBLIC_AST_FORMATTED_VALUE_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(formatted_values.clone());
    });
    PUBLIC_AST_JOINED_STR_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(joined_strs.clone());
    });
    PUBLIC_AST_TEMPLATE_STR_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(template_strs.clone());
    });
    PUBLIC_AST_COMPREHENSION_IS_ASYNC_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(comprehension_is_async.clone());
    });
    PUBLIC_AST_PATTERN_LIST_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(pattern_lists.clone());
    });
    PUBLIC_AST_EXPR_OPTION_LIST_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(expr_option_lists.clone());
    });
    PUBLIC_AST_EXPR_LIST_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(expr_lists.clone());
    });
    PUBLIC_AST_STMT_LIST_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(stmt_lists.clone());
    });
    PUBLIC_AST_EXCEPT_HANDLER_LIST_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(except_handler_lists.clone());
    });
    PUBLIC_AST_TYPE_PARAM_LIST_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(type_param_lists.clone());
    });
    PUBLIC_AST_MATCH_CLASS_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(match_classes.clone());
    });
    PUBLIC_AST_ANN_ASSIGN_SIMPLE_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(ann_assign_simple.clone());
    });
    PUBLIC_AST_ARG_TYPE_COMMENT_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(arg_type_comments.clone());
    });
    PUBLIC_AST_STMT_TYPE_COMMENT_OBJECTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(stmt_type_comments.clone());
    });
    let result = f();
    PUBLIC_AST_CONSTANT_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_INTERPOLATION_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_FORMATTED_VALUE_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_JOINED_STR_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_TEMPLATE_STR_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_COMPREHENSION_IS_ASYNC_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_PATTERN_LIST_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_EXPR_OPTION_LIST_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_EXPR_LIST_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_STMT_LIST_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_EXCEPT_HANDLER_LIST_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_TYPE_PARAM_LIST_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_MATCH_CLASS_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_ANN_ASSIGN_SIMPLE_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_ARG_TYPE_COMMENT_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_STMT_TYPE_COMMENT_OBJECTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    result
}

pub(super) fn public_ast_constant_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    node_index: ast::NodeIndex,
    range: TextRange,
) -> Option<PyObjectRef> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    let constant = PUBLIC_AST_CONSTANT_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|constants| constants.get(&node_index).cloned())
    })?;
    let node = NodeAst
        .into_ref_with_type(vm, pyast::NodeExprConstant::static_type().to_owned())
        .unwrap();
    let dict = node.as_object().dict().unwrap();
    dict.set_item("value", constant_data_to_object(vm, constant), vm)
        .unwrap();
    dict.set_item("kind", vm.ctx.none(), vm).unwrap();
    node_add_location(&dict, range, vm, source_file);
    Some(node.into())
}

pub(super) fn public_ast_interpolation_object(
    vm: &VirtualMachine,
    node_index: ast::NodeIndex,
) -> Option<(PyObjectRef, Option<Box<ast::Expr>>)> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    let interpolation = PUBLIC_AST_INTERPOLATION_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|interpolations| interpolations.get(&node_index).cloned())
    })?;
    Some((
        constant_data_to_object(vm, interpolation.str),
        interpolation.format_spec,
    ))
}

pub(super) fn public_ast_formatted_value_object(
    node_index: ast::NodeIndex,
) -> Option<PublicAstFormattedValue> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_FORMATTED_VALUE_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|formatted_values| formatted_values.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_joined_str_object(
    node_index: ast::NodeIndex,
) -> Option<PublicAstExprList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_JOINED_STR_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|joined_strs| joined_strs.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_template_str_object(
    node_index: ast::NodeIndex,
) -> Option<PublicAstExprList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_TEMPLATE_STR_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|template_strs| template_strs.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_comprehension_is_async_object(node_index: ast::NodeIndex) -> Option<i32> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_COMPREHENSION_IS_ASYNC_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).copied())
    })
}

pub(super) fn public_ast_pattern_list_object(
    node_index: ast::NodeIndex,
) -> Option<PublicAstPatternList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_PATTERN_LIST_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_expr_option_list_object(
    node_index: ast::NodeIndex,
) -> Option<PublicAstExprOptionList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_EXPR_OPTION_LIST_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_expr_list_object(
    node_index: ast::NodeIndex,
    field: PublicAstExprListField,
) -> Option<PublicAstExprOptionList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_EXPR_LIST_OBJECTS.with(|cell| {
        cell.borrow().as_ref().and_then(|values| {
            values
                .get(&node_index)
                .and_then(|values| values.get(field))
                .cloned()
        })
    })
}

pub(super) fn public_ast_stmt_list_object(
    node_index: ast::NodeIndex,
    field: PublicAstStmtListField,
) -> Option<PublicAstStmtList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_STMT_LIST_OBJECTS.with(|cell| {
        cell.borrow().as_ref().and_then(|values| {
            values
                .get(&node_index)
                .and_then(|values| values.get(field))
                .cloned()
        })
    })
}

pub(super) fn public_ast_except_handler_list_object(
    node_index: ast::NodeIndex,
) -> Option<PublicAstExceptHandlerList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_EXCEPT_HANDLER_LIST_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_type_param_list_object(
    node_index: ast::NodeIndex,
) -> Option<PublicAstTypeParamList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_TYPE_PARAM_LIST_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_match_class_object(
    node_index: ast::NodeIndex,
) -> Option<PublicAstMatchClass> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_MATCH_CLASS_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_ann_assign_simple_object(node_index: ast::NodeIndex) -> Option<i32> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_ANN_ASSIGN_SIMPLE_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).copied())
    })
}

pub(super) fn public_ast_arg_type_comment_object(
    node_index: ast::NodeIndex,
) -> Option<PyObjectRef> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_ARG_TYPE_COMMENT_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

pub(super) fn public_ast_stmt_type_comment_object(
    node_index: ast::NodeIndex,
) -> Option<PyObjectRef> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_STMT_TYPE_COMMENT_OBJECTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

fn constant_literal_to_constant_data(value: &ConstantLiteral) -> ConstantData {
    match value {
        ConstantLiteral::None => ConstantData::None,
        ConstantLiteral::Bool(value) => ConstantData::Boolean { value: *value },
        ConstantLiteral::Str { value, .. } => ConstantData::Str {
            value: value.as_ref().into(),
        },
        ConstantLiteral::Bytes(value) => ConstantData::Bytes {
            value: value.to_vec(),
        },
        ConstantLiteral::Int(value) => ConstantData::Integer {
            value: ruff_int_to_bigint(value).unwrap(),
        },
        ConstantLiteral::Tuple(value) => ConstantData::Tuple {
            elements: value
                .iter()
                .map(constant_literal_to_constant_data)
                .collect(),
        },
        ConstantLiteral::FrozenSet(value) => ConstantData::Frozenset {
            elements: value
                .iter()
                .map(constant_literal_to_constant_data)
                .collect(),
        },
        ConstantLiteral::Float(value) => ConstantData::Float { value: *value },
        ConstantLiteral::Complex { real, imag } => ConstantData::Complex {
            value: num_complex::Complex::new(*real, *imag),
        },
        ConstantLiteral::Ellipsis => ConstantData::Ellipsis,
    }
}

pub(super) fn constant_object_to_constant_data(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    value_object: PyObjectRef,
) -> PyResult<ConstantData> {
    let value = ConstantLiteral::ast_from_object(vm, source_file, value_object)?;
    Ok(constant_literal_to_constant_data(&value))
}

fn first_invalid_constant_type(vm: &VirtualMachine, value_object: PyObjectRef) -> PyResult<String> {
    let cls = value_object.class();
    let class_name = cls.name().to_owned();
    if cls.is(vm.ctx.types.tuple_type) {
        vm.with_recursion(" during compilation", || {
            let tuple = value_object.clone().downcast::<PyTuple>().map_err(|obj| {
                vm.new_type_error(format!(
                    "Expected type {}, not {}",
                    PyTuple::static_type().name(),
                    obj.class().name()
                ))
            })?;
            for item in tuple.iter() {
                if let Some(invalid_type) = first_invalid_constant_type_opt(vm, item.clone())? {
                    return Ok(invalid_type);
                }
            }
            Ok(class_name)
        })
    } else if cls.is(vm.ctx.types.frozenset_type) {
        vm.with_recursion(" during compilation", || {
            let set = value_object.clone().downcast::<PyFrozenSet>().unwrap();
            for item in set.elements() {
                if let Some(invalid_type) = first_invalid_constant_type_opt(vm, item)? {
                    return Ok(invalid_type);
                }
            }
            Ok(class_name)
        })
    } else {
        Ok(class_name)
    }
}

fn first_invalid_constant_type_opt(
    vm: &VirtualMachine,
    value_object: PyObjectRef,
) -> PyResult<Option<String>> {
    let cls = value_object.class();
    if cls.is(vm.ctx.types.none_type)
        || cls.is(vm.ctx.types.bool_type)
        || cls.is(vm.ctx.types.str_type)
        || cls.is(vm.ctx.types.bytes_type)
        || cls.is(vm.ctx.types.int_type)
        || cls.is(vm.ctx.types.float_type)
        || cls.is(vm.ctx.types.complex_type)
        || cls.is(vm.ctx.types.ellipsis_type)
    {
        return Ok(None);
    }
    if cls.is(vm.ctx.types.tuple_type) || cls.is(vm.ctx.types.frozenset_type) {
        return first_invalid_constant_type(vm, value_object).map(Some);
    }
    Ok(Some(cls.name().to_owned()))
}

fn constant_data_to_object(vm: &VirtualMachine, constant: ConstantData) -> PyObjectRef {
    match constant {
        ConstantData::None => vm.ctx.none(),
        ConstantData::Boolean { value } => vm.ctx.new_bool(value).to_pyobject(vm),
        ConstantData::Str { value } => vm.ctx.new_str(value.to_string()).to_pyobject(vm),
        ConstantData::Bytes { value } => vm.ctx.new_bytes(value).to_pyobject(vm),
        ConstantData::Integer { value } => vm.ctx.new_int(value).into(),
        ConstantData::Tuple { elements } => {
            let value = elements
                .into_iter()
                .map(|c| constant_data_to_object(vm, c))
                .collect();
            vm.ctx.new_tuple(value).to_pyobject(vm)
        }
        ConstantData::Frozenset { elements } => PyFrozenSet::from_iter(
            vm,
            elements.into_iter().map(|c| constant_data_to_object(vm, c)),
        )
        .unwrap()
        .into_pyobject(vm),
        ConstantData::Float { value } => vm.ctx.new_float(value).into_pyobject(vm),
        ConstantData::Complex { value } => vm.ctx.new_complex(value).into_pyobject(vm),
        ConstantData::Ellipsis => vm.ctx.ellipsis.clone().into(),
        ConstantData::Code { .. } | ConstantData::Slice { .. } => {
            unreachable!("public AST constants cannot contain code objects or slices")
        }
    }
}

// constructor
pub(super) fn constant_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<Constant> {
    let value_object = get_node_field(vm, &object, "value", "Constant")?;
    let (value, invalid_type) =
        match ConstantLiteral::ast_from_object(vm, source_file, value_object.clone()) {
            Ok(value) => (value, None),
            Err(_) => (
                ConstantLiteral::None,
                Some(first_invalid_constant_type(vm, value_object)?),
            ),
        };
    let _kind = get_ast_string_field_opt(vm, &object, "kind")?;

    Ok(Constant {
        range,
        value,
        invalid_type,
    })
}

impl Node for Constant {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            range,
            value,
            invalid_type: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprConstant::static_type().to_owned())
            .unwrap();
        let kind = match &value {
            ConstantLiteral::Str {
                prefix: StringLiteralPrefix::Unicode,
                ..
            } => vm.ctx.new_str("u").into(),
            _ => vm.ctx.none(),
        };
        let value = value.ast_to_object(vm, source_file);
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value, vm).unwrap();
        dict.set_item("kind", kind, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "Constant")?;
        constant_from_object_with_range(vm, source_file, object, range)
    }
}

impl Node for ConstantLiteral {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::None => vm.ctx.none(),
            Self::Bool(value) => vm.ctx.new_bool(value).to_pyobject(vm),
            Self::Str { value, .. } => vm.ctx.new_str(value).to_pyobject(vm),
            Self::Bytes(value) => vm.ctx.new_bytes(value.into()).to_pyobject(vm),
            Self::Int(value) => value.ast_to_object(vm, source_file),
            Self::Tuple(value) => {
                let value = value
                    .into_iter()
                    .map(|c| c.ast_to_object(vm, source_file))
                    .collect();
                vm.ctx.new_tuple(value).to_pyobject(vm)
            }
            Self::FrozenSet(value) => PyFrozenSet::from_iter(
                vm,
                value.into_iter().map(|c| c.ast_to_object(vm, source_file)),
            )
            .unwrap()
            .into_pyobject(vm),
            Self::Float(value) => vm.ctx.new_float(value).into_pyobject(vm),
            Self::Complex { real, imag } => vm
                .ctx
                .new_complex(num_complex::Complex::new(real, imag))
                .into_pyobject(vm),
            Self::Ellipsis => vm.ctx.ellipsis.clone().into(),
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        value_object: PyObjectRef,
    ) -> PyResult<Self> {
        let cls = value_object.class();
        let value = if cls.is(vm.ctx.types.none_type) {
            Self::None
        } else if cls.is(vm.ctx.types.bool_type) {
            Self::Bool(if value_object.is(&vm.ctx.true_value) {
                true
            } else if value_object.is(&vm.ctx.false_value) {
                false
            } else {
                value_object.try_to_value(vm)?
            })
        } else if cls.is(vm.ctx.types.str_type) {
            Self::Str {
                value: value_object.try_to_value::<String>(vm)?.into(),
                prefix: StringLiteralPrefix::Empty,
            }
        } else if cls.is(vm.ctx.types.bytes_type) {
            Self::Bytes(value_object.try_to_value::<Vec<u8>>(vm)?.into())
        } else if cls.is(vm.ctx.types.int_type) {
            Self::Int(Node::ast_from_object(vm, source_file, value_object)?)
        } else if cls.is(vm.ctx.types.tuple_type) {
            let tuple = value_object.downcast::<PyTuple>().map_err(|obj| {
                vm.new_type_error(format!(
                    "Expected type {}, not {}",
                    PyTuple::static_type().name(),
                    obj.class().name()
                ))
            })?;
            let tuple = tuple
                .into_iter()
                .map(|object| {
                    let object = object.clone();
                    vm.with_recursion("during compilation", || {
                        Node::ast_from_object(vm, source_file, object)
                    })
                })
                .collect::<PyResult<_>>()?;
            Self::Tuple(tuple)
        } else if cls.is(vm.ctx.types.frozenset_type) {
            let set = value_object.downcast::<PyFrozenSet>().unwrap();
            let elements = set
                .elements()
                .into_iter()
                .map(|object| {
                    vm.with_recursion("during compilation", || {
                        Node::ast_from_object(vm, source_file, object)
                    })
                })
                .collect::<PyResult<_>>()?;
            Self::FrozenSet(elements)
        } else if cls.is(vm.ctx.types.float_type) {
            let float = value_object.try_into_value(vm)?;
            Self::Float(float)
        } else if cls.is(vm.ctx.types.complex_type) {
            let complex = value_object.try_complex(vm)?;
            let complex = match complex {
                None => {
                    return Err(vm.new_type_error(format!(
                        "Expected type {}, not {}",
                        PyComplex::static_type().name(),
                        value_object.class().name()
                    )));
                }
                Some((value, _was_coerced)) => value,
            };
            Self::Complex {
                real: complex.re,
                imag: complex.im,
            }
        } else if cls.is(vm.ctx.types.ellipsis_type) {
            Self::Ellipsis
        } else {
            return Err(vm.new_type_error(format!(
                "got an invalid type in Constant: {}",
                value_object.class().name()
            )));
        };
        Ok(value)
    }
}

fn constant_to_ruff_expr(value: Constant) -> ast::Expr {
    let Constant {
        value,
        range,
        invalid_type: _,
    } = value;
    match value {
        ConstantLiteral::None => ast::Expr::NoneLiteral(ast::ExprNoneLiteral {
            node_index: Default::default(),
            range,
        }),
        ConstantLiteral::Bool(value) => ast::Expr::BooleanLiteral(ast::ExprBooleanLiteral {
            node_index: Default::default(),
            range,
            value,
        }),
        ConstantLiteral::Str { value, prefix } => {
            ast::Expr::StringLiteral(ast::ExprStringLiteral {
                node_index: Default::default(),
                range,
                value: ast::StringLiteralValue::single(ast::StringLiteral {
                    node_index: Default::default(),
                    range,
                    value,
                    flags: ast::StringLiteralFlags::empty().with_prefix(prefix),
                }),
            })
        }
        ConstantLiteral::Bytes(value) => {
            ast::Expr::BytesLiteral(ast::ExprBytesLiteral {
                node_index: Default::default(),
                range,
                value: ast::BytesLiteralValue::single(ast::BytesLiteral {
                    node_index: Default::default(),
                    range,
                    value,
                    flags: ast::BytesLiteralFlags::empty(), // TODO
                }),
            })
        }
        ConstantLiteral::Int(value) => ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
            node_index: Default::default(),
            range,
            value: ast::Number::Int(value),
        }),
        ConstantLiteral::Tuple(value) => ast::Expr::Tuple(ast::ExprTuple {
            node_index: Default::default(),
            range,
            elts: value
                .into_iter()
                .map(|value| {
                    constant_to_ruff_expr(Constant {
                        range: TextRange::default(),
                        value,
                        invalid_type: None,
                    })
                })
                .collect(),
            ctx: ast::ExprContext::Load,
            // TODO: Does this matter?
            parenthesized: true,
        }),
        ConstantLiteral::FrozenSet(value) => {
            let args = if value.is_empty() {
                Vec::new()
            } else {
                vec![ast::Expr::Set(ast::ExprSet {
                    node_index: Default::default(),
                    range: TextRange::default(),
                    elts: value
                        .into_iter()
                        .map(|value| {
                            constant_to_ruff_expr(Constant {
                                range: TextRange::default(),
                                value,
                                invalid_type: None,
                            })
                        })
                        .collect(),
                })]
            };
            ast::Expr::Call(ast::ExprCall {
                node_index: Default::default(),
                range,
                func: Box::new(ast::Expr::Name(ast::ExprName {
                    node_index: Default::default(),
                    range: TextRange::default(),
                    id: ast::name::Name::new_static("frozenset"),
                    ctx: ast::ExprContext::Load,
                })),
                arguments: ast::Arguments {
                    node_index: Default::default(),
                    range,
                    args: args.into(),
                    keywords: Default::default(),
                },
            })
        }
        ConstantLiteral::Float(value) => ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
            node_index: Default::default(),
            range,
            value: ast::Number::Float(value),
        }),
        ConstantLiteral::Complex { real, imag } => {
            ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
                node_index: Default::default(),
                range,
                value: ast::Number::Complex { real, imag },
            })
        }
        ConstantLiteral::Ellipsis => ast::Expr::EllipsisLiteral(ast::ExprEllipsisLiteral {
            node_index: Default::default(),
            range,
        }),
    }
}

pub(super) fn number_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprNumberLiteral,
) -> PyObjectRef {
    let ast::ExprNumberLiteral {
        node_index: _,
        range,
        value,
    } = constant;
    let c = match value {
        ast::Number::Int(n) => Constant::new_int(n, range),
        ast::Number::Float(n) => Constant::new_float(n, range),
        ast::Number::Complex { real, imag } => Constant::new_complex(real, imag, range),
    };
    c.ast_to_object(vm, source_file)
}

pub(super) fn string_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprStringLiteral,
) -> PyObjectRef {
    let ast::ExprStringLiteral {
        node_index: _,
        range,
        value,
    } = constant;
    let prefix = value
        .iter()
        .next()
        .map_or(StringLiteralPrefix::Empty, |part| part.flags.prefix());
    let c = Constant::new_str(value.to_str(), prefix, range);
    c.ast_to_object(vm, source_file)
}

pub(super) fn bytes_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprBytesLiteral,
) -> PyObjectRef {
    let ast::ExprBytesLiteral {
        node_index: _,
        range,
        value,
    } = constant;
    let bytes = value.as_slice().iter().flat_map(|b| b.value.iter());
    let c = Constant::new_bytes(bytes.copied().collect(), range);
    c.ast_to_object(vm, source_file)
}

pub(super) fn boolean_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprBooleanLiteral,
) -> PyObjectRef {
    let ast::ExprBooleanLiteral {
        node_index: _,
        range,
        value,
    } = constant;
    let c = Constant::new_bool(value, range);
    c.ast_to_object(vm, source_file)
}

pub(super) fn none_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprNoneLiteral,
) -> PyObjectRef {
    let ast::ExprNoneLiteral {
        node_index: _,
        range,
    } = constant;
    let c = Constant::new_none(range);
    c.ast_to_object(vm, source_file)
}

pub(super) fn ellipsis_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprEllipsisLiteral,
) -> PyObjectRef {
    let ast::ExprEllipsisLiteral {
        node_index: _,
        range,
    } = constant;
    let c = Constant::new_ellipsis(range);
    c.ast_to_object(vm, source_file)
}
