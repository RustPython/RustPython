#![allow(clippy::all)]

use super::*;
use crate::common::ascii;

macro_rules! impl_node {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident,
        [$($field:expr),* $(,)?],
        [$($attr:expr),* $(,)?]
    ) => {
        $(#[$meta])*
        $vis struct $name;

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
}

#[pyclass(module = "_ast", name = "mod", base = "NodeAst")]
pub(crate) struct NodeMod;

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeMod {}

impl_node!(
    #[pyclass(module = "_ast", name = "Module", base = "NodeMod")]
    pub(crate) struct NodeModModule,
    ["body", "type_ignores"], []
);

impl_node!(
    #[pyclass(module = "_ast", name = "Interactive", base = "NodeMod")]
    pub(crate) struct NodeModInteractive,
    ["body"], []
);

impl_node!(
    #[pyclass(module = "_ast", name = "Expression", base = "NodeMod")]
    pub(crate) struct NodeModExpression,
    ["body"], []
);

impl_node!(
    #[pyclass(module = "_ast", name = "FunctionType", base = "NodeMod")]
    pub(crate) struct NodeModFunctionType,
    ["argtypes", "returns"], []
);

#[pyclass(module = "_ast", name = "stmt", base = "NodeAst")]
pub(crate) struct NodeStmt;

#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmt {}

impl_node!(
    #[pyclass(module = "_ast", name = "FunctionDef", base = "NodeStmt")]
    pub(crate) struct NodeStmtFunctionDef,
    ["name", "args", "body", "decorator_list", "returns", "type_comment", "type_params"],
    ["lineno", "col_offset", "end_lineno", "end_col_offset"]
);

impl_node!(
    #[pyclass(module = "_ast", name = "AsyncFunctionDef", base = "NodeStmt")]
    pub(crate) struct NodeStmtAsyncFunctionDef,
    ["name", "args", "body", "decorator_list", "returns", "type_comment", "type_params"],
    ["lineno", "col_offset", "end_lineno", "end_col_offset"]
);

#[pyclass(module = "_ast", name = "ClassDef", base = "NodeStmt")]
pub(crate) struct NodeStmtClassDef;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtClassDef {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("name")).into(),
                ctx.new_str(ascii!("bases")).into(),
                ctx.new_str(ascii!("keywords")).into(),
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("decorator_list")).into(),
                ctx.new_str(ascii!("type_params")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Return", base = "NodeStmt")]
pub(crate) struct NodeStmtReturn;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtReturn {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("value")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Delete", base = "NodeStmt")]
pub(crate) struct NodeStmtDelete;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtDelete {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("targets")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Assign", base = "NodeStmt")]
pub(crate) struct NodeStmtAssign;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtAssign {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("targets")).into(),
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("type_comment")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "TypeAlias", base = "NodeStmt")]
pub(crate) struct NodeStmtTypeAlias;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtTypeAlias {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("name")).into(),
                ctx.new_str(ascii!("type_params")).into(),
                ctx.new_str(ascii!("value")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "AugAssign", base = "NodeStmt")]
pub(crate) struct NodeStmtAugAssign;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtAugAssign {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("target")).into(),
                ctx.new_str(ascii!("op")).into(),
                ctx.new_str(ascii!("value")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "AnnAssign", base = "NodeStmt")]
pub(crate) struct NodeStmtAnnAssign;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtAnnAssign {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("target")).into(),
                ctx.new_str(ascii!("annotation")).into(),
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("simple")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "For", base = "NodeStmt")]
pub(crate) struct NodeStmtFor;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtFor {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("target")).into(),
                ctx.new_str(ascii!("iter")).into(),
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("orelse")).into(),
                ctx.new_str(ascii!("type_comment")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "AsyncFor", base = "NodeStmt")]
pub(crate) struct NodeStmtAsyncFor;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtAsyncFor {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("target")).into(),
                ctx.new_str(ascii!("iter")).into(),
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("orelse")).into(),
                ctx.new_str(ascii!("type_comment")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "While", base = "NodeStmt")]
pub(crate) struct NodeStmtWhile;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtWhile {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("test")).into(),
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("orelse")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "If", base = "NodeStmt")]
pub(crate) struct NodeStmtIf;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtIf {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("test")).into(),
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("orelse")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "With", base = "NodeStmt")]
pub(crate) struct NodeStmtWith;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtWith {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("items")).into(),
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("type_comment")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "AsyncWith", base = "NodeStmt")]
pub(crate) struct NodeStmtAsyncWith;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtAsyncWith {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("items")).into(),
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("type_comment")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Match", base = "NodeStmt")]
pub(crate) struct NodeStmtMatch;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtMatch {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("subject")).into(),
                ctx.new_str(ascii!("cases")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Raise", base = "NodeStmt")]
pub(crate) struct NodeStmtRaise;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtRaise {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("exc")).into(),
                ctx.new_str(ascii!("cause")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Try", base = "NodeStmt")]
pub(crate) struct NodeStmtTry;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtTry {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("handlers")).into(),
                ctx.new_str(ascii!("orelse")).into(),
                ctx.new_str(ascii!("finalbody")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "TryStar", base = "NodeStmt")]
pub(crate) struct NodeStmtTryStar;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtTryStar {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("handlers")).into(),
                ctx.new_str(ascii!("orelse")).into(),
                ctx.new_str(ascii!("finalbody")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Assert", base = "NodeStmt")]
pub(crate) struct NodeStmtAssert;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtAssert {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("test")).into(),
                ctx.new_str(ascii!("msg")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Import", base = "NodeStmt")]
pub(crate) struct NodeStmtImport;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtImport {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("names")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "ImportFrom", base = "NodeStmt")]
pub(crate) struct NodeStmtImportFrom;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtImportFrom {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("module")).into(),
                ctx.new_str(ascii!("names")).into(),
                ctx.new_str(ascii!("level")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Global", base = "NodeStmt")]
pub(crate) struct NodeStmtGlobal;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtGlobal {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("names")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Nonlocal", base = "NodeStmt")]
pub(crate) struct NodeStmtNonlocal;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtNonlocal {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("names")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Expr", base = "NodeStmt")]
pub(crate) struct NodeStmtExpr;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtExpr {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("value")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Pass", base = "NodeStmt")]
pub(crate) struct NodeStmtPass;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtPass {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Break", base = "NodeStmt")]
pub(crate) struct NodeStmtBreak;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtBreak {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Continue", base = "NodeStmt")]
pub(crate) struct NodeStmtContinue;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeStmtContinue {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "expr", base = "NodeAst")]
pub(crate) struct NodeExpr;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExpr {}
#[pyclass(module = "_ast", name = "BoolOp", base = "NodeExpr")]
pub(crate) struct NodeExprBoolOp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprBoolOp {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("op")).into(),
                ctx.new_str(ascii!("values")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "NamedExpr", base = "NodeExpr")]
pub(crate) struct NodeExprNamedExpr;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprNamedExpr {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("target")).into(),
                ctx.new_str(ascii!("value")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "BinOp", base = "NodeExpr")]
pub(crate) struct NodeExprBinOp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprBinOp {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("left")).into(),
                ctx.new_str(ascii!("op")).into(),
                ctx.new_str(ascii!("right")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "UnaryOp", base = "NodeExpr")]
pub(crate) struct NodeExprUnaryOp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprUnaryOp {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("op")).into(),
                ctx.new_str(ascii!("operand")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Lambda", base = "NodeExpr")]
pub(crate) struct NodeExprLambda;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprLambda {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("args")).into(),
                ctx.new_str(ascii!("body")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "IfExp", base = "NodeExpr")]
pub(crate) struct NodeExprIfExp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprIfExp {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("test")).into(),
                ctx.new_str(ascii!("body")).into(),
                ctx.new_str(ascii!("orelse")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Dict", base = "NodeExpr")]
pub(crate) struct NodeExprDict;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprDict {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("keys")).into(),
                ctx.new_str(ascii!("values")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Set", base = "NodeExpr")]
pub(crate) struct NodeExprSet;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprSet {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("elts")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "ListComp", base = "NodeExpr")]
pub(crate) struct NodeExprListComp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprListComp {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("elt")).into(),
                ctx.new_str(ascii!("generators")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "SetComp", base = "NodeExpr")]
pub(crate) struct NodeExprSetComp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprSetComp {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("elt")).into(),
                ctx.new_str(ascii!("generators")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "DictComp", base = "NodeExpr")]
pub(crate) struct NodeExprDictComp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprDictComp {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("key")).into(),
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("generators")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "GeneratorExp", base = "NodeExpr")]
pub(crate) struct NodeExprGeneratorExp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprGeneratorExp {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("elt")).into(),
                ctx.new_str(ascii!("generators")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Await", base = "NodeExpr")]
pub(crate) struct NodeExprAwait;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprAwait {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("value")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Yield", base = "NodeExpr")]
pub(crate) struct NodeExprYield;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprYield {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("value")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "YieldFrom", base = "NodeExpr")]
pub(crate) struct NodeExprYieldFrom;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprYieldFrom {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("value")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Compare", base = "NodeExpr")]
pub(crate) struct NodeExprCompare;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprCompare {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("left")).into(),
                ctx.new_str(ascii!("ops")).into(),
                ctx.new_str(ascii!("comparators")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Call", base = "NodeExpr")]
pub(crate) struct NodeExprCall;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprCall {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("func")).into(),
                ctx.new_str(ascii!("args")).into(),
                ctx.new_str(ascii!("keywords")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "FormattedValue", base = "NodeExpr")]
pub(crate) struct NodeExprFormattedValue;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprFormattedValue {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("conversion")).into(),
                ctx.new_str(ascii!("format_spec")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "JoinedStr", base = "NodeExpr")]
pub(crate) struct NodeExprJoinedStr;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprJoinedStr {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("values")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Constant", base = "NodeExpr")]
pub(crate) struct NodeExprConstant;
#[pyclass(flags(HAS_DICT, BASETYPE))]
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
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Attribute", base = "NodeExpr")]
pub(crate) struct NodeExprAttribute;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprAttribute {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("attr")).into(),
                ctx.new_str(ascii!("ctx")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Subscript", base = "NodeExpr")]
pub(crate) struct NodeExprSubscript;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprSubscript {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("slice")).into(),
                ctx.new_str(ascii!("ctx")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Starred", base = "NodeExpr")]
pub(crate) struct NodeExprStarred;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprStarred {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("value")).into(),
                ctx.new_str(ascii!("ctx")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Name", base = "NodeExpr")]
pub(crate) struct NodeExprName;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprName {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("id")).into(),
                ctx.new_str(ascii!("ctx")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "List", base = "NodeExpr")]
pub(crate) struct NodeExprList;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprList {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("elts")).into(),
                ctx.new_str(ascii!("ctx")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Tuple", base = "NodeExpr")]
pub(crate) struct NodeExprTuple;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprTuple {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("elts")).into(),
                ctx.new_str(ascii!("ctx")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "Slice", base = "NodeExpr")]
pub(crate) struct NodeExprSlice;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprSlice {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("lower")).into(),
                ctx.new_str(ascii!("upper")).into(),
                ctx.new_str(ascii!("step")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "expr_context", base = "NodeAst")]
pub(crate) struct NodeExprContext;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprContext {}
#[pyclass(module = "_ast", name = "Load", base = "NodeExprContext")]
pub(crate) struct NodeExprContextLoad;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprContextLoad {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Store", base = "NodeExprContext")]
pub(crate) struct NodeExprContextStore;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprContextStore {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Del", base = "NodeExprContext")]
pub(crate) struct NodeExprContextDel;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExprContextDel {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "boolop", base = "NodeAst")]
pub(crate) struct NodeBoolOp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeBoolOp {}
#[pyclass(module = "_ast", name = "And", base = "NodeBoolOp")]
pub(crate) struct NodeBoolOpAnd;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeBoolOpAnd {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Or", base = "NodeBoolOp")]
pub(crate) struct NodeBoolOpOr;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeBoolOpOr {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "operator", base = "NodeAst")]
pub(crate) struct NodeOperator;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperator {}
#[pyclass(module = "_ast", name = "Add", base = "NodeOperator")]
pub(crate) struct NodeOperatorAdd;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorAdd {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Sub", base = "NodeOperator")]
pub(crate) struct NodeOperatorSub;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorSub {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Mult", base = "NodeOperator")]
pub(crate) struct NodeOperatorMult;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorMult {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "MatMult", base = "NodeOperator")]
pub(crate) struct NodeOperatorMatMult;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorMatMult {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Div", base = "NodeOperator")]
pub(crate) struct NodeOperatorDiv;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorDiv {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Mod", base = "NodeOperator")]
pub(crate) struct NodeOperatorMod;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorMod {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Pow", base = "NodeOperator")]
pub(crate) struct NodeOperatorPow;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorPow {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "LShift", base = "NodeOperator")]
pub(crate) struct NodeOperatorLShift;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorLShift {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "RShift", base = "NodeOperator")]
pub(crate) struct NodeOperatorRShift;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorRShift {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "BitOr", base = "NodeOperator")]
pub(crate) struct NodeOperatorBitOr;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorBitOr {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "BitXor", base = "NodeOperator")]
pub(crate) struct NodeOperatorBitXor;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorBitXor {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "BitAnd", base = "NodeOperator")]
pub(crate) struct NodeOperatorBitAnd;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorBitAnd {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "FloorDiv", base = "NodeOperator")]
pub(crate) struct NodeOperatorFloorDiv;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeOperatorFloorDiv {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "unaryop", base = "NodeAst")]
pub(crate) struct NodeUnaryOp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeUnaryOp {}
#[pyclass(module = "_ast", name = "Invert", base = "NodeUnaryOp")]
pub(crate) struct NodeUnaryOpInvert;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeUnaryOpInvert {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Not", base = "NodeUnaryOp")]
pub(crate) struct NodeUnaryOpNot;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeUnaryOpNot {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "UAdd", base = "NodeUnaryOp")]
pub(crate) struct NodeUnaryOpUAdd;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeUnaryOpUAdd {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "USub", base = "NodeUnaryOp")]
pub(crate) struct NodeUnaryOpUSub;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeUnaryOpUSub {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "cmpop", base = "NodeAst")]
pub(crate) struct NodeCmpOp;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOp {}
#[pyclass(module = "_ast", name = "Eq", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpEq;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpEq {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "NotEq", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpNotEq;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpNotEq {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Lt", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpLt;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpLt {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "LtE", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpLtE;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpLtE {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Gt", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpGt;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpGt {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "GtE", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpGtE;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpGtE {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "Is", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpIs;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpIs {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "IsNot", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpIsNot;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpIsNot {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "In", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpIn;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpIn {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "NotIn", base = "NodeCmpOp")]
pub(crate) struct NodeCmpOpNotIn;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeCmpOpNotIn {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(identifier!(ctx, _fields), ctx.new_tuple(vec![]).into());
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "comprehension", base = "NodeAst")]
pub(crate) struct NodeComprehension;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeComprehension {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("target")).into(),
                ctx.new_str(ascii!("iter")).into(),
                ctx.new_str(ascii!("ifs")).into(),
                ctx.new_str(ascii!("is_async")).into(),
            ])
            .into(),
        );
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "excepthandler", base = "NodeAst")]
pub(crate) struct NodeExceptHandler;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExceptHandler {}
#[pyclass(module = "_ast", name = "ExceptHandler", base = "NodeExceptHandler")]
pub(crate) struct NodeExceptHandlerExceptHandler;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeExceptHandlerExceptHandler {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("type")).into(),
                ctx.new_str(ascii!("name")).into(),
                ctx.new_str(ascii!("body")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "arguments", base = "NodeAst")]
pub(crate) struct NodeArguments;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeArguments {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("posonlyargs")).into(),
                ctx.new_str(ascii!("args")).into(),
                ctx.new_str(ascii!("vararg")).into(),
                ctx.new_str(ascii!("kwonlyargs")).into(),
                ctx.new_str(ascii!("kw_defaults")).into(),
                ctx.new_str(ascii!("kwarg")).into(),
                ctx.new_str(ascii!("defaults")).into(),
            ])
            .into(),
        );
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "arg", base = "NodeAst")]
pub(crate) struct NodeArg;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeArg {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("arg")).into(),
                ctx.new_str(ascii!("annotation")).into(),
                ctx.new_str(ascii!("type_comment")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "keyword", base = "NodeAst")]
pub(crate) struct NodeKeyword;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeKeyword {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("arg")).into(),
                ctx.new_str(ascii!("value")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "alias", base = "NodeAst")]
pub(crate) struct NodeAlias;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeAlias {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("name")).into(),
                ctx.new_str(ascii!("asname")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "withitem", base = "NodeAst")]
pub(crate) struct NodeWithItem;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeWithItem {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("context_expr")).into(),
                ctx.new_str(ascii!("optional_vars")).into(),
            ])
            .into(),
        );
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "match_case", base = "NodeAst")]
pub(crate) struct NodeMatchCase;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeMatchCase {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("pattern")).into(),
                ctx.new_str(ascii!("guard")).into(),
                ctx.new_str(ascii!("body")).into(),
            ])
            .into(),
        );
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "pattern", base = "NodeAst")]
pub(crate) struct NodePattern;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePattern {}
#[pyclass(module = "_ast", name = "MatchValue", base = "NodePattern")]
pub(crate) struct NodePatternMatchValue;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePatternMatchValue {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("value")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "MatchSingleton", base = "NodePattern")]
pub(crate) struct NodePatternMatchSingleton;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePatternMatchSingleton {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("value")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "MatchSequence", base = "NodePattern")]
pub(crate) struct NodePatternMatchSequence;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePatternMatchSequence {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("patterns")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "MatchMapping", base = "NodePattern")]
pub(crate) struct NodePatternMatchMapping;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePatternMatchMapping {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("keys")).into(),
                ctx.new_str(ascii!("patterns")).into(),
                ctx.new_str(ascii!("rest")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "MatchClass", base = "NodePattern")]
pub(crate) struct NodePatternMatchClass;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePatternMatchClass {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("cls")).into(),
                ctx.new_str(ascii!("patterns")).into(),
                ctx.new_str(ascii!("kwd_attrs")).into(),
                ctx.new_str(ascii!("kwd_patterns")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "MatchStar", base = "NodePattern")]
pub(crate) struct NodePatternMatchStar;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePatternMatchStar {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("name")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "MatchAs", base = "NodePattern")]
pub(crate) struct NodePatternMatchAs;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePatternMatchAs {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("pattern")).into(),
                ctx.new_str(ascii!("name")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "MatchOr", base = "NodePattern")]
pub(crate) struct NodePatternMatchOr;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodePatternMatchOr {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("patterns")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "type_ignore", base = "NodeAst")]
pub(crate) struct NodeTypeIgnore;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeTypeIgnore {}
#[pyclass(module = "_ast", name = "TypeIgnore", base = "NodeTypeIgnore")]
pub(crate) struct NodeTypeIgnoreTypeIgnore;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeTypeIgnoreTypeIgnore {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("tag")).into(),
            ])
            .into(),
        );
        class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
    }
}
#[pyclass(module = "_ast", name = "type_param", base = "NodeAst")]
pub(crate) struct NodeTypeParam;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeTypeParam {}
#[pyclass(module = "_ast", name = "TypeVar", base = "NodeTypeParam")]
pub(crate) struct NodeTypeParamTypeVar;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeTypeParamTypeVar {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![
                ctx.new_str(ascii!("name")).into(),
                ctx.new_str(ascii!("bound")).into(),
            ])
            .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "ParamSpec", base = "NodeTypeParam")]
pub(crate) struct NodeTypeParamParamSpec;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeTypeParamParamSpec {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("name")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}
#[pyclass(module = "_ast", name = "TypeVarTuple", base = "NodeTypeParam")]
pub(crate) struct NodeTypeParamTypeVarTuple;
#[pyclass(flags(HAS_DICT, BASETYPE))]
impl NodeTypeParamTypeVarTuple {
    #[extend_class]
    fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
        class.set_attr(
            identifier!(ctx, _fields),
            ctx.new_tuple(vec![ctx.new_str(ascii!("name")).into()])
                .into(),
        );
        class.set_attr(
            identifier!(ctx, _attributes),
            ctx.new_list(vec![
                ctx.new_str(ascii!("lineno")).into(),
                ctx.new_str(ascii!("col_offset")).into(),
                ctx.new_str(ascii!("end_lineno")).into(),
                ctx.new_str(ascii!("end_col_offset")).into(),
            ])
            .into(),
        );
    }
}

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
