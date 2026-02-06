#![allow(clippy::all)]

use super::*;
use crate::builtins::{PyGenericAlias, PyTuple, PyTupleRef, PyTypeRef, make_union};
use crate::common::ascii;
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
use crate::types::Initializer;

macro_rules! impl_node {
    (
        #[pyclass(module = $_mod:literal, name = $_name:literal, base = $base:ty)]
        $vis:vis struct $name:ident,
        fields: [$($field:expr),* $(,)?],
        attributes: [$($attr:expr),* $(,)?] $(,)?
    ) => {
        #[pyclass(module = $_mod, name = $_name, base = $base)]
        #[repr(transparent)]
        $vis struct $name($base);

        impl_base_node!($name, fields: [$($field),*], attributes: [$($attr),*]);
    };
    // Without attributes
    (
        #[pyclass(module = $_mod:literal, name = $_name:literal, base = $base:ty)]
        $vis:vis struct $name:ident,
        fields: [$($field:expr),* $(,)?] $(,)?
    ) => {
        impl_node!(
            #[pyclass(module = $_mod, name = $_name, base = $base)]
            $vis struct $name,
            fields: [$($field),*],
            attributes: [],
        );
    };
    // Without fields
    (
        #[pyclass(module = $_mod:literal, name = $_name:literal, base = $base:ty)]
        $vis:vis struct $name:ident,
        attributes: [$($attr:expr),* $(,)?] $(,)?
    ) => {
        impl_node!(
            #[pyclass(module = $_mod, name = $_name, base = $base)]
            $vis struct $name,
            fields: [],
            attributes: [$($attr),*],
        );
    };
    // Without fields and attributes
    (
        #[pyclass(module = $_mod:literal, name = $_name:literal, base = $base:ty)]
        $vis:vis struct $name:ident $(,)?
    ) => {
        impl_node!(
            #[pyclass(module = $_mod, name = $_name, base = $base)]
            $vis struct $name,
            fields: [],
            attributes: [],
        );
    };
}

macro_rules! impl_base_node {
    // Base node without fields/attributes (e.g. NodeMod, NodeExpr)
    ($name:ident) => {
        #[pyclass(flags(HAS_DICT, BASETYPE))]
        impl $name {
            #[pymethod]
            fn __reduce__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
                super::python::_ast::ast_reduce(zelf, vm)
            }

            #[pymethod]
            fn __replace__(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
                super::python::_ast::ast_replace(zelf, args, vm)
            }

            #[extend_class]
            fn extend_class(ctx: &Context, class: &'static Py<PyType>) {
                class.set_attr(
                    identifier!(ctx, _attributes),
                    ctx.empty_tuple.clone().into(),
                );
            }
        }
    };
    // Leaf node with fields and attributes
    ($name:ident, fields: [$($field:expr),*], attributes: [$($attr:expr),*]) => {
        #[pyclass(flags(HAS_DICT, BASETYPE))]
        impl $name {
            #[pymethod]
            fn __reduce__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
                super::python::_ast::ast_reduce(zelf, vm)
            }

            #[pymethod]
            fn __replace__(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
                super::python::_ast::ast_replace(zelf, args, vm)
            }

            #[extend_class]
            fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
                class.set_attr(
                    identifier!(ctx, _fields),
                    ctx.new_tuple(vec![
                        $(
                            ctx.new_str(ascii!($field)).into()
                        ),*
                    ])
                    .into(),
                );

                class.set_str_attr(
                    "__match_args__",
                    ctx.new_tuple(vec![
                        $(
                            ctx.new_str(ascii!($field)).into()
                        ),*
                    ]),
                    ctx,
                );

                class.set_attr(
                    identifier!(ctx, _attributes),
                    ctx.new_tuple(vec![
                        $(
                            ctx.new_str(ascii!($attr)).into()
                        ),*
                    ])
                    .into(),
                );

                // Signal that this is a built-in AST node with field defaults
                class.set_attr(
                    ctx.intern_str("_field_types"),
                    ctx.new_dict().into(),
                );
            }
        }
    };
}

#[pyclass(module = "_ast", name = "mod", base = NodeAst)]
pub(crate) struct NodeMod(NodeAst);

impl_base_node!(NodeMod);

impl_node!(
    #[pyclass(module = "_ast", name = "Module", base = NodeMod)]
    pub(crate) struct NodeModModule,
    fields: ["body", "type_ignores"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Interactive", base = NodeMod)]
    pub(crate) struct NodeModInteractive,
    fields: ["body"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Expression", base = NodeMod)]
    pub(crate) struct NodeModExpression,
    fields: ["body"],
);

#[pyclass(module = "_ast", name = "stmt", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeStmt(NodeAst);

impl_base_node!(NodeStmt);

impl_node!(
    #[pyclass(module = "_ast", name = "FunctionType", base = NodeMod)]
    pub(crate) struct NodeModFunctionType,
    fields: ["argtypes", "returns"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "FunctionDef", base = NodeStmt)]
    pub(crate) struct NodeStmtFunctionDef,
    fields: ["name", "args", "body", "decorator_list", "returns", "type_comment", "type_params"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "AsyncFunctionDef", base = NodeStmt)]
    pub(crate) struct NodeStmtAsyncFunctionDef,
    fields: ["name", "args", "body", "decorator_list", "returns", "type_comment", "type_params"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "ClassDef", base = NodeStmt)]
    pub(crate) struct NodeStmtClassDef,
    fields: ["name", "bases", "keywords", "body", "decorator_list", "type_params"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Return", base = NodeStmt)]
    pub(crate) struct NodeStmtReturn,
    fields: ["value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Delete", base = NodeStmt)]
    pub(crate) struct NodeStmtDelete,
    fields: ["targets"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Assign", base = NodeStmt)]
    pub(crate) struct NodeStmtAssign,
    fields: ["targets", "value", "type_comment"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "TypeAlias", base = NodeStmt)]
    pub(crate) struct NodeStmtTypeAlias,
    fields: ["name", "type_params", "value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "AugAssign", base = NodeStmt)]
    pub(crate) struct NodeStmtAugAssign,
    fields: ["target", "op", "value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "AnnAssign", base = NodeStmt)]
    pub(crate) struct NodeStmtAnnAssign,
    fields: ["target", "annotation", "value", "simple"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "For", base = NodeStmt)]
    pub(crate) struct NodeStmtFor,
    fields: ["target", "iter", "body", "orelse", "type_comment"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "AsyncFor", base = NodeStmt)]
    pub(crate) struct NodeStmtAsyncFor,
    fields: ["target", "iter", "body", "orelse", "type_comment"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "While", base = NodeStmt)]
    pub(crate) struct NodeStmtWhile,
    fields: ["test", "body", "orelse"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "If", base = NodeStmt)]
    pub(crate) struct NodeStmtIf,
    fields: ["test", "body", "orelse"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "With", base = NodeStmt)]
    pub(crate) struct NodeStmtWith,
    fields: ["items", "body", "type_comment"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "AsyncWith", base = NodeStmt)]
    pub(crate) struct NodeStmtAsyncWith,
    fields: ["items", "body", "type_comment"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Match", base = NodeStmt)]
    pub(crate) struct NodeStmtMatch,
    fields: ["subject", "cases"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Raise", base = NodeStmt)]
    pub(crate) struct NodeStmtRaise,
    fields: ["exc", "cause"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Try", base = NodeStmt)]
    pub(crate) struct NodeStmtTry,
    fields: ["body", "handlers", "orelse", "finalbody"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "TryStar", base = NodeStmt)]
    pub(crate) struct NodeStmtTryStar,
    fields: ["body", "handlers", "orelse", "finalbody"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Assert", base = NodeStmt)]
    pub(crate) struct NodeStmtAssert,
    fields: ["test", "msg"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Import", base = NodeStmt)]
    pub(crate) struct NodeStmtImport,
    fields: ["names"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "ImportFrom", base = NodeStmt)]
    pub(crate) struct NodeStmtImportFrom,
    fields: ["module", "names", "level"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Global", base = NodeStmt)]
    pub(crate) struct NodeStmtGlobal,
    fields: ["names"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Nonlocal", base = NodeStmt)]
    pub(crate) struct NodeStmtNonlocal,
    fields: ["names"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Expr", base = NodeStmt)]
    pub(crate) struct NodeStmtExpr,
    fields: ["value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Pass", base = NodeStmt)]
    pub(crate) struct NodeStmtPass,
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Break", base = NodeStmt)]
    pub(crate) struct NodeStmtBreak,
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

#[pyclass(module = "_ast", name = "expr", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeExpr(NodeAst);

impl_base_node!(NodeExpr);

impl_node!(
    #[pyclass(module = "_ast", name = "Continue", base = NodeStmt)]
    pub(crate) struct NodeStmtContinue,
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "BoolOp", base = NodeExpr)]
    pub(crate) struct NodeExprBoolOp,
    fields: ["op", "values"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "NamedExpr", base = NodeExpr)]
    pub(crate) struct NodeExprNamedExpr,
    fields: ["target", "value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "BinOp", base = NodeExpr)]
    pub(crate) struct NodeExprBinOp,
    fields: ["left", "op", "right"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "UnaryOp", base = NodeExpr)]
    pub(crate) struct NodeExprUnaryOp,
    fields: ["op", "operand"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Lambda", base = NodeExpr)]
    pub(crate) struct NodeExprLambda,
    fields: ["args", "body"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "IfExp", base = NodeExpr)]
    pub(crate) struct NodeExprIfExp,
    fields: ["test", "body", "orelse"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Dict", base = NodeExpr)]
    pub(crate) struct NodeExprDict,
    fields: ["keys", "values"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Set", base = NodeExpr)]
    pub(crate) struct NodeExprSet,
    fields: ["elts"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "ListComp", base = NodeExpr)]
    pub(crate) struct NodeExprListComp,
    fields: ["elt", "generators"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "SetComp", base = NodeExpr)]
    pub(crate) struct NodeExprSetComp,
    fields: ["elt", "generators"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "DictComp", base = NodeExpr)]
    pub(crate) struct NodeExprDictComp,
    fields: ["key", "value", "generators"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "GeneratorExp", base = NodeExpr)]
    pub(crate) struct NodeExprGeneratorExp,
    fields: ["elt", "generators"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Await", base = NodeExpr)]
    pub(crate) struct NodeExprAwait,
    fields: ["value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Yield", base = NodeExpr)]
    pub(crate) struct NodeExprYield,
    fields: ["value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "YieldFrom", base = NodeExpr)]
    pub(crate) struct NodeExprYieldFrom,
    fields: ["value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Compare", base = NodeExpr)]
    pub(crate) struct NodeExprCompare,
    fields: ["left", "ops", "comparators"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Call", base = NodeExpr)]
    pub(crate) struct NodeExprCall,
    fields: ["func", "args", "keywords"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "FormattedValue", base = NodeExpr)]
    pub(crate) struct NodeExprFormattedValue,
    fields: ["value", "conversion", "format_spec"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "JoinedStr", base = NodeExpr)]
    pub(crate) struct NodeExprJoinedStr,
    fields: ["values"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "TemplateStr", base = NodeExpr)]
    pub(crate) struct NodeExprTemplateStr,
    fields: ["values"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Interpolation", base = NodeExpr)]
    pub(crate) struct NodeExprInterpolation,
    fields: ["value", "str", "conversion", "format_spec"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

// NodeExprConstant needs custom Initializer to default kind to None
#[pyclass(module = "_ast", name = "Constant", base = NodeExpr)]
#[repr(transparent)]
pub(crate) struct NodeExprConstant(NodeExpr);

#[pyclass(flags(HAS_DICT, BASETYPE), with(Initializer))]
impl NodeExprConstant {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("kind")).into(),
            ])
            .into(),
        );

        class.set_str_attr(
            "__match_args__",
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("kind")).into(),
            ]),
            ctx,
        );

        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}

impl Initializer for NodeExprConstant {
    type Args = FuncArgs;

    fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        <NodeAst as Initializer>::slot_init(zelf.clone(), args, vm)?;
        // kind defaults to None if not provided
        let dict = zelf.as_object().dict().unwrap();
        if !dict.contains_key("kind", vm) {
            dict.set_item("kind", vm.ctx.none(), vm)?;
        }
        Ok(())
    }

    fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
        unreachable!("slot_init is defined")
    }
}

impl_node!(
    #[pyclass(module = "_ast", name = "Attribute", base = NodeExpr)]
    pub(crate) struct NodeExprAttribute,
    fields: ["value", "attr", "ctx"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Subscript", base = NodeExpr)]
    pub(crate) struct NodeExprSubscript,
    fields: ["value", "slice", "ctx"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Starred", base = NodeExpr)]
    pub(crate) struct NodeExprStarred,
    fields: ["value", "ctx"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Name", base = NodeExpr)]
    pub(crate) struct NodeExprName,
    fields: ["id", "ctx"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "List", base = NodeExpr)]
    pub(crate) struct NodeExprList,
    fields: ["elts", "ctx"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Tuple", base = NodeExpr)]
    pub(crate) struct NodeExprTuple,
    fields: ["elts", "ctx"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

#[pyclass(module = "_ast", name = "expr_context", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeExprContext(NodeAst);

impl_base_node!(NodeExprContext);

impl_node!(
    #[pyclass(module = "_ast", name = "Slice", base = NodeExpr)]
    pub(crate) struct NodeExprSlice,
    fields: ["lower", "upper", "step"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "Load", base = NodeExprContext)]
    pub(crate) struct NodeExprContextLoad,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Store", base = NodeExprContext)]
    pub(crate) struct NodeExprContextStore,
);

#[pyclass(module = "_ast", name = "boolop", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeBoolOp(NodeAst);

impl_base_node!(NodeBoolOp);

impl_node!(
    #[pyclass(module = "_ast", name = "Del", base = NodeExprContext)]
    pub(crate) struct NodeExprContextDel,
);

impl_node!(
    #[pyclass(module = "_ast", name = "And", base = NodeBoolOp)]
    pub(crate) struct NodeBoolOpAnd,
);

#[pyclass(module = "_ast", name = "operator", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeOperator(NodeAst);

impl_base_node!(NodeOperator);

impl_node!(
    #[pyclass(module = "_ast", name = "Or", base = NodeBoolOp)]
    pub(crate) struct NodeBoolOpOr,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Add", base = NodeOperator)]
    pub(crate) struct NodeOperatorAdd,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Sub", base = NodeOperator)]
    pub(crate) struct NodeOperatorSub,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Mult", base = NodeOperator)]
    pub(crate) struct NodeOperatorMult,
);

impl_node!(
    #[pyclass(module = "_ast", name = "MatMult", base = NodeOperator)]
    pub(crate) struct NodeOperatorMatMult,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Div", base = NodeOperator)]
    pub(crate) struct NodeOperatorDiv,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Mod", base = NodeOperator)]
    pub(crate) struct NodeOperatorMod,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Pow", base = NodeOperator)]
    pub(crate) struct NodeOperatorPow,
);

impl_node!(
    #[pyclass(module = "_ast", name = "LShift", base = NodeOperator)]
    pub(crate) struct NodeOperatorLShift,
);

impl_node!(
    #[pyclass(module = "_ast", name = "RShift", base = NodeOperator)]
    pub(crate) struct NodeOperatorRShift,
);

impl_node!(
    #[pyclass(module = "_ast", name = "BitOr", base = NodeOperator)]
    pub(crate) struct NodeOperatorBitOr,
);

impl_node!(
    #[pyclass(module = "_ast", name = "BitXor", base = NodeOperator)]
    pub(crate) struct NodeOperatorBitXor,
);

impl_node!(
    #[pyclass(module = "_ast", name = "BitAnd", base = NodeOperator)]
    pub(crate) struct NodeOperatorBitAnd,
);

#[pyclass(module = "_ast", name = "unaryop", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeUnaryOp(NodeAst);

impl_base_node!(NodeUnaryOp);

impl_node!(
    #[pyclass(module = "_ast", name = "FloorDiv", base = NodeOperator)]
    pub(crate) struct NodeOperatorFloorDiv,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Invert", base = NodeUnaryOp)]
    pub(crate) struct NodeUnaryOpInvert,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Not", base = NodeUnaryOp)]
    pub(crate) struct NodeUnaryOpNot,
);

impl_node!(
    #[pyclass(module = "_ast", name = "UAdd", base = NodeUnaryOp)]
    pub(crate) struct NodeUnaryOpUAdd,
);

#[pyclass(module = "_ast", name = "cmpop", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeCmpOp(NodeAst);

impl_base_node!(NodeCmpOp);

impl_node!(
    #[pyclass(module = "_ast", name = "USub", base = NodeUnaryOp)]
    pub(crate) struct NodeUnaryOpUSub,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Eq", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpEq,
);

impl_node!(
    #[pyclass(module = "_ast", name = "NotEq", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpNotEq,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Lt", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpLt,
);

impl_node!(
    #[pyclass(module = "_ast", name = "LtE", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpLtE,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Gt", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpGt,
);

impl_node!(
    #[pyclass(module = "_ast", name = "GtE", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpGtE,
);

impl_node!(
    #[pyclass(module = "_ast", name = "Is", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpIs,
);

impl_node!(
    #[pyclass(module = "_ast", name = "IsNot", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpIsNot,
);

impl_node!(
    #[pyclass(module = "_ast", name = "In", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpIn,
);

impl_node!(
    #[pyclass(module = "_ast", name = "NotIn", base = NodeCmpOp)]
    pub(crate) struct NodeCmpOpNotIn,
);

#[pyclass(module = "_ast", name = "excepthandler", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeExceptHandler(NodeAst);

impl_base_node!(NodeExceptHandler);

impl_node!(
    #[pyclass(module = "_ast", name = "comprehension", base = NodeAst)]
    pub(crate) struct NodeComprehension,
    fields: ["target", "iter", "ifs", "is_async"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "ExceptHandler", base = NodeExceptHandler)]
    pub(crate) struct NodeExceptHandlerExceptHandler,
    fields: ["type", "name", "body"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "arguments", base = NodeAst)]
    pub(crate) struct NodeArguments,
    fields: ["posonlyargs", "args", "vararg", "kwonlyargs", "kw_defaults", "kwarg", "defaults"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "arg", base = NodeAst)]
    pub(crate) struct NodeArg,
    fields: ["arg", "annotation", "type_comment"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "keyword", base = NodeAst)]
    pub(crate) struct NodeKeyword,
    fields: ["arg", "value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "alias", base = NodeAst)]
    pub(crate) struct NodeAlias,
    fields: ["name", "asname"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "withitem", base = NodeAst)]
    pub(crate) struct NodeWithItem,
    fields: ["context_expr", "optional_vars"],
);

#[pyclass(module = "_ast", name = "pattern", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodePattern(NodeAst);

impl_base_node!(NodePattern);

impl_node!(
    #[pyclass(module = "_ast", name = "match_case", base = NodeAst)]
    pub(crate) struct NodeMatchCase,
    fields: ["pattern", "guard", "body"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "MatchValue", base = NodePattern)]
    pub(crate) struct NodePatternMatchValue,
    fields: ["value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "MatchSingleton", base = NodePattern)]
    pub(crate) struct NodePatternMatchSingleton,
    fields: ["value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "MatchSequence", base = NodePattern)]
    pub(crate) struct NodePatternMatchSequence,
    fields: ["patterns"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "MatchMapping", base = NodePattern)]
    pub(crate) struct NodePatternMatchMapping,
    fields: ["keys", "patterns", "rest"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "MatchClass", base = NodePattern)]
    pub(crate) struct NodePatternMatchClass,
    fields: ["cls", "patterns", "kwd_attrs", "kwd_patterns"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "MatchStar", base = NodePattern)]
    pub(crate) struct NodePatternMatchStar,
    fields: ["name"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "MatchAs", base = NodePattern)]
    pub(crate) struct NodePatternMatchAs,
    fields: ["pattern", "name"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

#[pyclass(module = "_ast", name = "type_ignore", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeTypeIgnore(NodeAst);

impl_base_node!(NodeTypeIgnore);

impl_node!(
    #[pyclass(module = "_ast", name = "MatchOr", base = NodePattern)]
    pub(crate) struct NodePatternMatchOr,
    fields: ["patterns"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

#[pyclass(module = "_ast", name = "type_param", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeTypeParam(NodeAst);

impl_base_node!(NodeTypeParam);

impl_node!(
    #[pyclass(module = "_ast", name = "TypeIgnore", base = NodeTypeIgnore)]
    pub(crate) struct NodeTypeIgnoreTypeIgnore,
    fields: ["lineno", "tag"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "TypeVar", base = NodeTypeParam)]
    pub(crate) struct NodeTypeParamTypeVar,
    fields: ["name", "bound", "default_value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "ParamSpec", base = NodeTypeParam)]
    pub(crate) struct NodeTypeParamParamSpec,
    fields: ["name", "default_value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "TypeVarTuple", base = NodeTypeParam)]
    pub(crate) struct NodeTypeParamTypeVarTuple,
    fields: ["name", "default_value"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

/// Marker for how to resolve an ASDL field type into a Python type object.
#[derive(Clone, Copy)]
enum FieldType {
    /// AST node type reference (e.g. "expr", "stmt")
    Node(&'static str),
    /// Built-in type reference (e.g. "str", "int", "object")
    Builtin(&'static str),
    /// list[NodeType] — Py_GenericAlias(list, node_type)
    ListOf(&'static str),
    /// list[BuiltinType] — Py_GenericAlias(list, builtin_type)
    ListOfBuiltin(&'static str),
    /// NodeType | None — Union[node_type, None]
    Optional(&'static str),
    /// BuiltinType | None — Union[builtin_type, None]
    OptionalBuiltin(&'static str),
}

/// Field type annotations for all concrete AST node classes.
/// Derived from add_ast_annotations() in Python-ast.c.
const FIELD_TYPES: &[(&str, &[(&str, FieldType)])] = &[
    // -- mod --
    (
        "Module",
        &[
            ("body", FieldType::ListOf("stmt")),
            ("type_ignores", FieldType::ListOf("type_ignore")),
        ],
    ),
    ("Interactive", &[("body", FieldType::ListOf("stmt"))]),
    ("Expression", &[("body", FieldType::Node("expr"))]),
    (
        "FunctionType",
        &[
            ("argtypes", FieldType::ListOf("expr")),
            ("returns", FieldType::Node("expr")),
        ],
    ),
    // -- stmt --
    (
        "FunctionDef",
        &[
            ("name", FieldType::Builtin("str")),
            ("args", FieldType::Node("arguments")),
            ("body", FieldType::ListOf("stmt")),
            ("decorator_list", FieldType::ListOf("expr")),
            ("returns", FieldType::Optional("expr")),
            ("type_comment", FieldType::OptionalBuiltin("str")),
            ("type_params", FieldType::ListOf("type_param")),
        ],
    ),
    (
        "AsyncFunctionDef",
        &[
            ("name", FieldType::Builtin("str")),
            ("args", FieldType::Node("arguments")),
            ("body", FieldType::ListOf("stmt")),
            ("decorator_list", FieldType::ListOf("expr")),
            ("returns", FieldType::Optional("expr")),
            ("type_comment", FieldType::OptionalBuiltin("str")),
            ("type_params", FieldType::ListOf("type_param")),
        ],
    ),
    (
        "ClassDef",
        &[
            ("name", FieldType::Builtin("str")),
            ("bases", FieldType::ListOf("expr")),
            ("keywords", FieldType::ListOf("keyword")),
            ("body", FieldType::ListOf("stmt")),
            ("decorator_list", FieldType::ListOf("expr")),
            ("type_params", FieldType::ListOf("type_param")),
        ],
    ),
    ("Return", &[("value", FieldType::Optional("expr"))]),
    ("Delete", &[("targets", FieldType::ListOf("expr"))]),
    (
        "Assign",
        &[
            ("targets", FieldType::ListOf("expr")),
            ("value", FieldType::Node("expr")),
            ("type_comment", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "TypeAlias",
        &[
            ("name", FieldType::Node("expr")),
            ("type_params", FieldType::ListOf("type_param")),
            ("value", FieldType::Node("expr")),
        ],
    ),
    (
        "AugAssign",
        &[
            ("target", FieldType::Node("expr")),
            ("op", FieldType::Node("operator")),
            ("value", FieldType::Node("expr")),
        ],
    ),
    (
        "AnnAssign",
        &[
            ("target", FieldType::Node("expr")),
            ("annotation", FieldType::Node("expr")),
            ("value", FieldType::Optional("expr")),
            ("simple", FieldType::Builtin("int")),
        ],
    ),
    (
        "For",
        &[
            ("target", FieldType::Node("expr")),
            ("iter", FieldType::Node("expr")),
            ("body", FieldType::ListOf("stmt")),
            ("orelse", FieldType::ListOf("stmt")),
            ("type_comment", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "AsyncFor",
        &[
            ("target", FieldType::Node("expr")),
            ("iter", FieldType::Node("expr")),
            ("body", FieldType::ListOf("stmt")),
            ("orelse", FieldType::ListOf("stmt")),
            ("type_comment", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "While",
        &[
            ("test", FieldType::Node("expr")),
            ("body", FieldType::ListOf("stmt")),
            ("orelse", FieldType::ListOf("stmt")),
        ],
    ),
    (
        "If",
        &[
            ("test", FieldType::Node("expr")),
            ("body", FieldType::ListOf("stmt")),
            ("orelse", FieldType::ListOf("stmt")),
        ],
    ),
    (
        "With",
        &[
            ("items", FieldType::ListOf("withitem")),
            ("body", FieldType::ListOf("stmt")),
            ("type_comment", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "AsyncWith",
        &[
            ("items", FieldType::ListOf("withitem")),
            ("body", FieldType::ListOf("stmt")),
            ("type_comment", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "Match",
        &[
            ("subject", FieldType::Node("expr")),
            ("cases", FieldType::ListOf("match_case")),
        ],
    ),
    (
        "Raise",
        &[
            ("exc", FieldType::Optional("expr")),
            ("cause", FieldType::Optional("expr")),
        ],
    ),
    (
        "Try",
        &[
            ("body", FieldType::ListOf("stmt")),
            ("handlers", FieldType::ListOf("excepthandler")),
            ("orelse", FieldType::ListOf("stmt")),
            ("finalbody", FieldType::ListOf("stmt")),
        ],
    ),
    (
        "TryStar",
        &[
            ("body", FieldType::ListOf("stmt")),
            ("handlers", FieldType::ListOf("excepthandler")),
            ("orelse", FieldType::ListOf("stmt")),
            ("finalbody", FieldType::ListOf("stmt")),
        ],
    ),
    (
        "Assert",
        &[
            ("test", FieldType::Node("expr")),
            ("msg", FieldType::Optional("expr")),
        ],
    ),
    ("Import", &[("names", FieldType::ListOf("alias"))]),
    (
        "ImportFrom",
        &[
            ("module", FieldType::OptionalBuiltin("str")),
            ("names", FieldType::ListOf("alias")),
            ("level", FieldType::OptionalBuiltin("int")),
        ],
    ),
    ("Global", &[("names", FieldType::ListOfBuiltin("str"))]),
    ("Nonlocal", &[("names", FieldType::ListOfBuiltin("str"))]),
    ("Expr", &[("value", FieldType::Node("expr"))]),
    // -- expr --
    (
        "BoolOp",
        &[
            ("op", FieldType::Node("boolop")),
            ("values", FieldType::ListOf("expr")),
        ],
    ),
    (
        "NamedExpr",
        &[
            ("target", FieldType::Node("expr")),
            ("value", FieldType::Node("expr")),
        ],
    ),
    (
        "BinOp",
        &[
            ("left", FieldType::Node("expr")),
            ("op", FieldType::Node("operator")),
            ("right", FieldType::Node("expr")),
        ],
    ),
    (
        "UnaryOp",
        &[
            ("op", FieldType::Node("unaryop")),
            ("operand", FieldType::Node("expr")),
        ],
    ),
    (
        "Lambda",
        &[
            ("args", FieldType::Node("arguments")),
            ("body", FieldType::Node("expr")),
        ],
    ),
    (
        "IfExp",
        &[
            ("test", FieldType::Node("expr")),
            ("body", FieldType::Node("expr")),
            ("orelse", FieldType::Node("expr")),
        ],
    ),
    (
        "Dict",
        &[
            ("keys", FieldType::ListOf("expr")),
            ("values", FieldType::ListOf("expr")),
        ],
    ),
    ("Set", &[("elts", FieldType::ListOf("expr"))]),
    (
        "ListComp",
        &[
            ("elt", FieldType::Node("expr")),
            ("generators", FieldType::ListOf("comprehension")),
        ],
    ),
    (
        "SetComp",
        &[
            ("elt", FieldType::Node("expr")),
            ("generators", FieldType::ListOf("comprehension")),
        ],
    ),
    (
        "DictComp",
        &[
            ("key", FieldType::Node("expr")),
            ("value", FieldType::Node("expr")),
            ("generators", FieldType::ListOf("comprehension")),
        ],
    ),
    (
        "GeneratorExp",
        &[
            ("elt", FieldType::Node("expr")),
            ("generators", FieldType::ListOf("comprehension")),
        ],
    ),
    ("Await", &[("value", FieldType::Node("expr"))]),
    ("Yield", &[("value", FieldType::Optional("expr"))]),
    ("YieldFrom", &[("value", FieldType::Node("expr"))]),
    (
        "Compare",
        &[
            ("left", FieldType::Node("expr")),
            ("ops", FieldType::ListOf("cmpop")),
            ("comparators", FieldType::ListOf("expr")),
        ],
    ),
    (
        "Call",
        &[
            ("func", FieldType::Node("expr")),
            ("args", FieldType::ListOf("expr")),
            ("keywords", FieldType::ListOf("keyword")),
        ],
    ),
    (
        "FormattedValue",
        &[
            ("value", FieldType::Node("expr")),
            ("conversion", FieldType::Builtin("int")),
            ("format_spec", FieldType::Optional("expr")),
        ],
    ),
    ("JoinedStr", &[("values", FieldType::ListOf("expr"))]),
    ("TemplateStr", &[("values", FieldType::ListOf("expr"))]),
    (
        "Interpolation",
        &[
            ("value", FieldType::Node("expr")),
            ("str", FieldType::Builtin("object")),
            ("conversion", FieldType::Builtin("int")),
            ("format_spec", FieldType::Optional("expr")),
        ],
    ),
    (
        "Constant",
        &[
            ("value", FieldType::Builtin("object")),
            ("kind", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "Attribute",
        &[
            ("value", FieldType::Node("expr")),
            ("attr", FieldType::Builtin("str")),
            ("ctx", FieldType::Node("expr_context")),
        ],
    ),
    (
        "Subscript",
        &[
            ("value", FieldType::Node("expr")),
            ("slice", FieldType::Node("expr")),
            ("ctx", FieldType::Node("expr_context")),
        ],
    ),
    (
        "Starred",
        &[
            ("value", FieldType::Node("expr")),
            ("ctx", FieldType::Node("expr_context")),
        ],
    ),
    (
        "Name",
        &[
            ("id", FieldType::Builtin("str")),
            ("ctx", FieldType::Node("expr_context")),
        ],
    ),
    (
        "List",
        &[
            ("elts", FieldType::ListOf("expr")),
            ("ctx", FieldType::Node("expr_context")),
        ],
    ),
    (
        "Tuple",
        &[
            ("elts", FieldType::ListOf("expr")),
            ("ctx", FieldType::Node("expr_context")),
        ],
    ),
    (
        "Slice",
        &[
            ("lower", FieldType::Optional("expr")),
            ("upper", FieldType::Optional("expr")),
            ("step", FieldType::Optional("expr")),
        ],
    ),
    // -- misc --
    (
        "comprehension",
        &[
            ("target", FieldType::Node("expr")),
            ("iter", FieldType::Node("expr")),
            ("ifs", FieldType::ListOf("expr")),
            ("is_async", FieldType::Builtin("int")),
        ],
    ),
    (
        "ExceptHandler",
        &[
            ("type", FieldType::Optional("expr")),
            ("name", FieldType::OptionalBuiltin("str")),
            ("body", FieldType::ListOf("stmt")),
        ],
    ),
    (
        "arguments",
        &[
            ("posonlyargs", FieldType::ListOf("arg")),
            ("args", FieldType::ListOf("arg")),
            ("vararg", FieldType::Optional("arg")),
            ("kwonlyargs", FieldType::ListOf("arg")),
            ("kw_defaults", FieldType::ListOf("expr")),
            ("kwarg", FieldType::Optional("arg")),
            ("defaults", FieldType::ListOf("expr")),
        ],
    ),
    (
        "arg",
        &[
            ("arg", FieldType::Builtin("str")),
            ("annotation", FieldType::Optional("expr")),
            ("type_comment", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "keyword",
        &[
            ("arg", FieldType::OptionalBuiltin("str")),
            ("value", FieldType::Node("expr")),
        ],
    ),
    (
        "alias",
        &[
            ("name", FieldType::Builtin("str")),
            ("asname", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "withitem",
        &[
            ("context_expr", FieldType::Node("expr")),
            ("optional_vars", FieldType::Optional("expr")),
        ],
    ),
    (
        "match_case",
        &[
            ("pattern", FieldType::Node("pattern")),
            ("guard", FieldType::Optional("expr")),
            ("body", FieldType::ListOf("stmt")),
        ],
    ),
    // -- pattern --
    ("MatchValue", &[("value", FieldType::Node("expr"))]),
    ("MatchSingleton", &[("value", FieldType::Builtin("object"))]),
    (
        "MatchSequence",
        &[("patterns", FieldType::ListOf("pattern"))],
    ),
    (
        "MatchMapping",
        &[
            ("keys", FieldType::ListOf("expr")),
            ("patterns", FieldType::ListOf("pattern")),
            ("rest", FieldType::OptionalBuiltin("str")),
        ],
    ),
    (
        "MatchClass",
        &[
            ("cls", FieldType::Node("expr")),
            ("patterns", FieldType::ListOf("pattern")),
            ("kwd_attrs", FieldType::ListOfBuiltin("str")),
            ("kwd_patterns", FieldType::ListOf("pattern")),
        ],
    ),
    ("MatchStar", &[("name", FieldType::OptionalBuiltin("str"))]),
    (
        "MatchAs",
        &[
            ("pattern", FieldType::Optional("pattern")),
            ("name", FieldType::OptionalBuiltin("str")),
        ],
    ),
    ("MatchOr", &[("patterns", FieldType::ListOf("pattern"))]),
    // -- type_ignore --
    (
        "TypeIgnore",
        &[
            ("lineno", FieldType::Builtin("int")),
            ("tag", FieldType::Builtin("str")),
        ],
    ),
    // -- type_param --
    (
        "TypeVar",
        &[
            ("name", FieldType::Builtin("str")),
            ("bound", FieldType::Optional("expr")),
            ("default_value", FieldType::Optional("expr")),
        ],
    ),
    (
        "ParamSpec",
        &[
            ("name", FieldType::Builtin("str")),
            ("default_value", FieldType::Optional("expr")),
        ],
    ),
    (
        "TypeVarTuple",
        &[
            ("name", FieldType::Builtin("str")),
            ("default_value", FieldType::Optional("expr")),
        ],
    ),
];

pub fn extend_module_nodes(vm: &VirtualMachine, module: &Py<PyModule>) {
    extend_module!(vm, module, {
        "AST" => NodeAst::make_class(&vm.ctx),
        "mod" => NodeMod::make_class(&vm.ctx),
        "Module" => NodeModModule::make_class(&vm.ctx),
        "Interactive" => NodeModInteractive::make_class(&vm.ctx),
        "Expression" => NodeModExpression::make_class(&vm.ctx),
        "FunctionType" => NodeModFunctionType::make_class(&vm.ctx),
        "stmt" => NodeStmt::make_class(&vm.ctx),
        "FunctionDef" => NodeStmtFunctionDef::make_class(&vm.ctx),
        "AsyncFunctionDef" => NodeStmtAsyncFunctionDef::make_class(&vm.ctx),
        "ClassDef" => NodeStmtClassDef::make_class(&vm.ctx),
        "Return" => NodeStmtReturn::make_class(&vm.ctx),
        "Delete" => NodeStmtDelete::make_class(&vm.ctx),
        "Assign" => NodeStmtAssign::make_class(&vm.ctx),
        "TypeAlias" => NodeStmtTypeAlias::make_class(&vm.ctx),
        "AugAssign" => NodeStmtAugAssign::make_class(&vm.ctx),
        "AnnAssign" => NodeStmtAnnAssign::make_class(&vm.ctx),
        "For" => NodeStmtFor::make_class(&vm.ctx),
        "AsyncFor" => NodeStmtAsyncFor::make_class(&vm.ctx),
        "While" => NodeStmtWhile::make_class(&vm.ctx),
        "If" => NodeStmtIf::make_class(&vm.ctx),
        "With" => NodeStmtWith::make_class(&vm.ctx),
        "AsyncWith" => NodeStmtAsyncWith::make_class(&vm.ctx),
        "Match" => NodeStmtMatch::make_class(&vm.ctx),
        "Raise" => NodeStmtRaise::make_class(&vm.ctx),
        "Try" => NodeStmtTry::make_class(&vm.ctx),
        "TryStar" => NodeStmtTryStar::make_class(&vm.ctx),
        "Assert" => NodeStmtAssert::make_class(&vm.ctx),
        "Import" => NodeStmtImport::make_class(&vm.ctx),
        "ImportFrom" => NodeStmtImportFrom::make_class(&vm.ctx),
        "Global" => NodeStmtGlobal::make_class(&vm.ctx),
        "Nonlocal" => NodeStmtNonlocal::make_class(&vm.ctx),
        "Expr" => NodeStmtExpr::make_class(&vm.ctx),
        "Pass" => NodeStmtPass::make_class(&vm.ctx),
        "Break" => NodeStmtBreak::make_class(&vm.ctx),
        "Continue" => NodeStmtContinue::make_class(&vm.ctx),
        "expr" => NodeExpr::make_class(&vm.ctx),
        "BoolOp" => NodeExprBoolOp::make_class(&vm.ctx),
        "NamedExpr" => NodeExprNamedExpr::make_class(&vm.ctx),
        "BinOp" => NodeExprBinOp::make_class(&vm.ctx),
        "UnaryOp" => NodeExprUnaryOp::make_class(&vm.ctx),
        "Lambda" => NodeExprLambda::make_class(&vm.ctx),
        "IfExp" => NodeExprIfExp::make_class(&vm.ctx),
        "Dict" => NodeExprDict::make_class(&vm.ctx),
        "Set" => NodeExprSet::make_class(&vm.ctx),
        "ListComp" => NodeExprListComp::make_class(&vm.ctx),
        "SetComp" => NodeExprSetComp::make_class(&vm.ctx),
        "DictComp" => NodeExprDictComp::make_class(&vm.ctx),
        "GeneratorExp" => NodeExprGeneratorExp::make_class(&vm.ctx),
        "Await" => NodeExprAwait::make_class(&vm.ctx),
        "Yield" => NodeExprYield::make_class(&vm.ctx),
        "YieldFrom" => NodeExprYieldFrom::make_class(&vm.ctx),
        "Compare" => NodeExprCompare::make_class(&vm.ctx),
        "Call" => NodeExprCall::make_class(&vm.ctx),
        "FormattedValue" => NodeExprFormattedValue::make_class(&vm.ctx),
        "JoinedStr" => NodeExprJoinedStr::make_class(&vm.ctx),
        "TemplateStr" => NodeExprTemplateStr::make_class(&vm.ctx),
        "Interpolation" => NodeExprInterpolation::make_class(&vm.ctx),
        "Constant" => NodeExprConstant::make_class(&vm.ctx),
        "Attribute" => NodeExprAttribute::make_class(&vm.ctx),
        "Subscript" => NodeExprSubscript::make_class(&vm.ctx),
        "Starred" => NodeExprStarred::make_class(&vm.ctx),
        "Name" => NodeExprName::make_class(&vm.ctx),
        "List" => NodeExprList::make_class(&vm.ctx),
        "Tuple" => NodeExprTuple::make_class(&vm.ctx),
        "Slice" => NodeExprSlice::make_class(&vm.ctx),
        "expr_context" => NodeExprContext::make_class(&vm.ctx),
        "Load" => NodeExprContextLoad::make_class(&vm.ctx),
        "Store" => NodeExprContextStore::make_class(&vm.ctx),
        "Del" => NodeExprContextDel::make_class(&vm.ctx),
        "boolop" => NodeBoolOp::make_class(&vm.ctx),
        "And" => NodeBoolOpAnd::make_class(&vm.ctx),
        "Or" => NodeBoolOpOr::make_class(&vm.ctx),
        "operator" => NodeOperator::make_class(&vm.ctx),
        "Add" => NodeOperatorAdd::make_class(&vm.ctx),
        "Sub" => NodeOperatorSub::make_class(&vm.ctx),
        "Mult" => NodeOperatorMult::make_class(&vm.ctx),
        "MatMult" => NodeOperatorMatMult::make_class(&vm.ctx),
        "Div" => NodeOperatorDiv::make_class(&vm.ctx),
        "Mod" => NodeOperatorMod::make_class(&vm.ctx),
        "Pow" => NodeOperatorPow::make_class(&vm.ctx),
        "LShift" => NodeOperatorLShift::make_class(&vm.ctx),
        "RShift" => NodeOperatorRShift::make_class(&vm.ctx),
        "BitOr" => NodeOperatorBitOr::make_class(&vm.ctx),
        "BitXor" => NodeOperatorBitXor::make_class(&vm.ctx),
        "BitAnd" => NodeOperatorBitAnd::make_class(&vm.ctx),
        "FloorDiv" => NodeOperatorFloorDiv::make_class(&vm.ctx),
        "unaryop" => NodeUnaryOp::make_class(&vm.ctx),
        "Invert" => NodeUnaryOpInvert::make_class(&vm.ctx),
        "Not" => NodeUnaryOpNot::make_class(&vm.ctx),
        "UAdd" => NodeUnaryOpUAdd::make_class(&vm.ctx),
        "USub" => NodeUnaryOpUSub::make_class(&vm.ctx),
        "cmpop" => NodeCmpOp::make_class(&vm.ctx),
        "Eq" => NodeCmpOpEq::make_class(&vm.ctx),
        "NotEq" => NodeCmpOpNotEq::make_class(&vm.ctx),
        "Lt" => NodeCmpOpLt::make_class(&vm.ctx),
        "LtE" => NodeCmpOpLtE::make_class(&vm.ctx),
        "Gt" => NodeCmpOpGt::make_class(&vm.ctx),
        "GtE" => NodeCmpOpGtE::make_class(&vm.ctx),
        "Is" => NodeCmpOpIs::make_class(&vm.ctx),
        "IsNot" => NodeCmpOpIsNot::make_class(&vm.ctx),
        "In" => NodeCmpOpIn::make_class(&vm.ctx),
        "NotIn" => NodeCmpOpNotIn::make_class(&vm.ctx),
        "comprehension" => NodeComprehension::make_class(&vm.ctx),
        "excepthandler" => NodeExceptHandler::make_class(&vm.ctx),
        "ExceptHandler" => NodeExceptHandlerExceptHandler::make_class(&vm.ctx),
        "arguments" => NodeArguments::make_class(&vm.ctx),
        "arg" => NodeArg::make_class(&vm.ctx),
        "keyword" => NodeKeyword::make_class(&vm.ctx),
        "alias" => NodeAlias::make_class(&vm.ctx),
        "withitem" => NodeWithItem::make_class(&vm.ctx),
        "match_case" => NodeMatchCase::make_class(&vm.ctx),
        "pattern" => NodePattern::make_class(&vm.ctx),
        "MatchValue" => NodePatternMatchValue::make_class(&vm.ctx),
        "MatchSingleton" => NodePatternMatchSingleton::make_class(&vm.ctx),
        "MatchSequence" => NodePatternMatchSequence::make_class(&vm.ctx),
        "MatchMapping" => NodePatternMatchMapping::make_class(&vm.ctx),
        "MatchClass" => NodePatternMatchClass::make_class(&vm.ctx),
        "MatchStar" => NodePatternMatchStar::make_class(&vm.ctx),
        "MatchAs" => NodePatternMatchAs::make_class(&vm.ctx),
        "MatchOr" => NodePatternMatchOr::make_class(&vm.ctx),
        "type_ignore" => NodeTypeIgnore::make_class(&vm.ctx),
        "TypeIgnore" => NodeTypeIgnoreTypeIgnore::make_class(&vm.ctx),
        "type_param" => NodeTypeParam::make_class(&vm.ctx),
        "TypeVar" => NodeTypeParamTypeVar::make_class(&vm.ctx),
        "ParamSpec" => NodeTypeParamParamSpec::make_class(&vm.ctx),
        "TypeVarTuple" => NodeTypeParamTypeVarTuple::make_class(&vm.ctx),
    });

    // Populate _field_types with real Python type objects
    populate_field_types(vm, module);
    populate_singletons(vm, module);
    force_ast_module_name(vm, module);
    populate_repr(vm, module);
}

fn populate_field_types(vm: &VirtualMachine, module: &Py<PyModule>) {
    let list_type: PyTypeRef = vm.ctx.types.list_type.to_owned();
    let none_type: PyObjectRef = vm.ctx.types.none_type.to_owned().into();

    // Resolve a builtin type name to a Python type object
    let resolve_builtin = |name: &str| -> PyObjectRef {
        let ty: &Py<PyType> = match name {
            "str" => vm.ctx.types.str_type,
            "int" => vm.ctx.types.int_type,
            "object" => vm.ctx.types.object_type,
            "bool" => vm.ctx.types.bool_type,
            _ => unreachable!("unknown builtin type: {name}"),
        };
        ty.to_owned().into()
    };

    // Resolve an AST node type name by looking it up from the module
    let resolve_node = |name: &str| -> PyObjectRef {
        module
            .get_attr(vm.ctx.intern_str(name), vm)
            .unwrap_or_else(|_| panic!("AST node type '{name}' not found in module"))
    };

    let field_types_attr = vm.ctx.intern_str("_field_types");
    let annotations_attr = vm.ctx.intern_str("__annotations__");
    let empty_dict: PyObjectRef = vm.ctx.new_dict().into();

    for &(class_name, fields) in FIELD_TYPES {
        if fields.is_empty() {
            continue;
        }

        let class = module
            .get_attr(class_name, vm)
            .unwrap_or_else(|_| panic!("AST class '{class_name}' not found in module"));
        let dict = vm.ctx.new_dict();

        for &(field_name, ref field_type) in fields {
            let type_obj = match field_type {
                FieldType::Node(name) => resolve_node(name),
                FieldType::Builtin(name) => resolve_builtin(name),
                FieldType::ListOf(name) => {
                    let elem = resolve_node(name);
                    let args = PyTuple::new_ref(vec![elem], &vm.ctx);
                    PyGenericAlias::new(list_type.clone(), args, false, vm).to_pyobject(vm)
                }
                FieldType::ListOfBuiltin(name) => {
                    let elem = resolve_builtin(name);
                    let args = PyTuple::new_ref(vec![elem], &vm.ctx);
                    PyGenericAlias::new(list_type.clone(), args, false, vm).to_pyobject(vm)
                }
                FieldType::Optional(name) => {
                    let base = resolve_node(name);
                    let union_args = PyTuple::new_ref(vec![base, none_type.clone()], &vm.ctx);
                    make_union(&union_args, vm).expect("failed to create union type")
                }
                FieldType::OptionalBuiltin(name) => {
                    let base = resolve_builtin(name);
                    let union_args = PyTuple::new_ref(vec![base, none_type.clone()], &vm.ctx);
                    make_union(&union_args, vm).expect("failed to create union type")
                }
            };
            dict.set_item(vm.ctx.intern_str(field_name), type_obj, vm)
                .expect("failed to set field type");
        }

        let dict_obj: PyObjectRef = dict.into();
        if let Some(type_obj) = class.downcast_ref::<PyType>() {
            type_obj.set_attr(field_types_attr, dict_obj.clone());
            type_obj.set_attr(annotations_attr, dict_obj);

            // Set None as class-level default for optional fields.
            // When ast_type_init skips optional fields, the instance
            // inherits None from the class (init_types in Python-ast.c).
            let none = vm.ctx.none();
            for &(field_name, ref field_type) in fields {
                if matches!(
                    field_type,
                    FieldType::Optional(_) | FieldType::OptionalBuiltin(_)
                ) {
                    type_obj.set_attr(vm.ctx.intern_str(field_name), none.clone());
                }
            }
        }
    }

    // CPython sets __annotations__ for all built-in AST node classes, even
    // when _field_types is an empty dict (e.g., operators, Load/Store/Del).
    for (_name, value) in &module.dict() {
        let Some(type_obj) = value.downcast_ref::<PyType>() else {
            continue;
        };
        if let Some(field_types) = type_obj.get_attr(field_types_attr) {
            type_obj.set_attr(annotations_attr, field_types);
        }
    }

    // Base AST classes (e.g., expr, stmt) should still expose __annotations__.
    const BASE_AST_TYPES: &[&str] = &[
        "mod",
        "stmt",
        "expr",
        "expr_context",
        "boolop",
        "operator",
        "unaryop",
        "cmpop",
        "excepthandler",
        "pattern",
        "type_ignore",
        "type_param",
    ];
    for &class_name in BASE_AST_TYPES {
        let class = module
            .get_attr(class_name, vm)
            .unwrap_or_else(|_| panic!("AST class '{class_name}' not found in module"));
        let Some(type_obj) = class.downcast_ref::<PyType>() else {
            continue;
        };
        if type_obj.get_attr(field_types_attr).is_none() {
            type_obj.set_attr(field_types_attr, empty_dict.clone());
        }
        if type_obj.get_attr(annotations_attr).is_none() {
            type_obj.set_attr(annotations_attr, empty_dict.clone());
        }
    }
}

fn populate_singletons(vm: &VirtualMachine, module: &Py<PyModule>) {
    let instance_attr = vm.ctx.intern_str("_instance");
    const SINGLETON_TYPES: &[&str] = &[
        // expr_context
        "Load", "Store", "Del", // boolop
        "And", "Or", // operator
        "Add", "Sub", "Mult", "MatMult", "Div", "Mod", "Pow", "LShift", "RShift", "BitOr",
        "BitXor", "BitAnd", "FloorDiv", // unaryop
        "Invert", "Not", "UAdd", "USub", // cmpop
        "Eq", "NotEq", "Lt", "LtE", "Gt", "GtE", "Is", "IsNot", "In", "NotIn",
    ];

    for &class_name in SINGLETON_TYPES {
        let class = module
            .get_attr(class_name, vm)
            .unwrap_or_else(|_| panic!("AST class '{class_name}' not found in module"));
        let Some(type_obj) = class.downcast_ref::<PyType>() else {
            continue;
        };
        let instance = vm
            .ctx
            .new_base_object(type_obj.to_owned(), Some(vm.ctx.new_dict()));
        type_obj.set_attr(instance_attr, instance);
    }
}

fn force_ast_module_name(vm: &VirtualMachine, module: &Py<PyModule>) {
    let ast_name = vm.ctx.new_str("ast");
    for (_name, value) in &module.dict() {
        let Some(type_obj) = value.downcast_ref::<PyType>() else {
            continue;
        };
        type_obj.set_attr(identifier!(vm, __module__), ast_name.clone().into());
    }
}

fn populate_repr(_vm: &VirtualMachine, module: &Py<PyModule>) {
    for (_name, value) in &module.dict() {
        let Some(type_obj) = value.downcast_ref::<PyType>() else {
            continue;
        };
        type_obj
            .slots
            .repr
            .store(Some(super::python::_ast::ast_repr));
    }
}
