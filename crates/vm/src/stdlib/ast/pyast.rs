#![allow(clippy::all)]

use super::*;
use crate::common::ascii;

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

        #[pyclass(flags(HAS_DICT, BASETYPE))]
        impl $name {
            #[extend_class]
            fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
                class.set_attr(
                    identifier!(ctx, _fields),
                    ctx.new_tuple(vec![
                        $(
                            ctx.new_str(ascii!($field)).into()
                        ),*
                    ]).into(),
                );

                class.set_attr(
                    identifier!(ctx, _attributes),
                    ctx.new_list(vec![
                        $(
                            ctx.new_str(ascii!($attr)).into()
                        ),*
                    ]).into(),
                );
            }
        }
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

#[pyclass(module = "_ast", name = "mod", base = NodeAst)]
pub(crate) struct NodeMod(NodeAst);

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeMod {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmt {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExpr {}

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

impl_node!(
    #[pyclass(module = "_ast", name = "Constant", base = NodeExpr)]
    pub(crate) struct NodeExprConstant,
    fields: ["value", "kind"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprContext {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeBoolOp {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperator {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeUnaryOp {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOp {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExceptHandler {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePattern {}

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

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeTypeIgnore {}

impl_node!(
    #[pyclass(module = "_ast", name = "MatchOr", base = NodePattern)]
    pub(crate) struct NodePatternMatchOr,
    fields: ["patterns"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

#[pyclass(module = "_ast", name = "type_param", base = NodeAst)]
#[repr(transparent)]
pub(crate) struct NodeTypeParam(NodeAst);

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeTypeParam {}

impl_node!(
    #[pyclass(module = "_ast", name = "TypeIgnore", base = NodeTypeIgnore)]
    pub(crate) struct NodeTypeIgnoreTypeIgnore,
    fields: ["lineno", "tag"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "TypeVar", base = NodeTypeParam)]
    pub(crate) struct NodeTypeParamTypeVar,
    fields: ["name", "bound"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "ParamSpec", base = NodeTypeParam)]
    pub(crate) struct NodeTypeParamParamSpec,
    fields: ["name"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

impl_node!(
    #[pyclass(module = "_ast", name = "TypeVarTuple", base = NodeTypeParam)]
    pub(crate) struct NodeTypeParamTypeVarTuple,
    fields: ["name"],
    attributes: ["lineno", "col_offset", "end_lineno", "end_col_offset"],
);

pub fn extend_module_nodes(vm: &VirtualMachine, module: &Py<PyModule>) {
    extend_module!(vm, module, {
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
    })
}
