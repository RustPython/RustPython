use ruff_python_ast::{
    self as ruff, Arguments, BoolOp, Comprehension, ConversionFlag, Expr, Identifier, Operator,
    Parameter, ParameterWithDefault, Parameters,
};
use ruff_text_size::Ranged;
use rustpython_compiler_core::SourceFile;
use rustpython_literal::escape::{AsciiEscape, UnicodeEscape};
use std::fmt::{self, Display as _};

mod precedence {
    macro_rules! precedence {
        ($($op:ident,)*) => {
            precedence!(@0, $($op,)*);
        };
        (@$i:expr, $op1:ident, $($op:ident,)*) => {
            pub const $op1: u8 = $i;
            precedence!(@$i + 1, $($op,)*);
        };
        (@$i:expr,) => {};
    }
    precedence!(
        TUPLE, TEST, OR, AND, NOT, CMP, // "EXPR" =
        BOR, BXOR, BAND, SHIFT, ARITH, TERM, FACTOR, POWER, AWAIT, ATOM,
    );
    pub const EXPR: u8 = BOR;
}

struct Unparser<'a, 'b, 'c> {
    f: &'b mut fmt::Formatter<'a>,
    source: &'c SourceFile,
}

impl<'a, 'b, 'c> Unparser<'a, 'b, 'c> {
    const fn new(f: &'b mut fmt::Formatter<'a>, source: &'c SourceFile) -> Self {
        Unparser { f, source }
    }

    fn p(&mut self, s: &str) -> fmt::Result {
        self.f.write_str(s)
    }

    fn p_id(&mut self, s: &Identifier) -> fmt::Result {
        self.f.write_str(s.as_str())
    }

    fn p_if(&mut self, cond: bool, s: &str) -> fmt::Result {
        if cond {
            self.f.write_str(s)?;
        }
        Ok(())
    }

    fn p_delim(&mut self, first: &mut bool, s: &str) -> fmt::Result {
        self.p_if(!std::mem::take(first), s)
    }

    fn write_fmt(&mut self, f: fmt::Arguments<'_>) -> fmt::Result {
        self.f.write_fmt(f)
    }

    fn unparse_expr(&mut self, ast: &Expr, level: u8) -> fmt::Result {
        macro_rules! op_prec {
            ($op_ty:ident, $x:expr, $enu:path, $($var:ident($op:literal, $prec:ident)),*$(,)?) => {
                match $x {
                    $(<$enu>::$var => (op_prec!(@space $op_ty, $op), precedence::$prec),)*
                }
            };
            (@space bin, $op:literal) => {
                concat!(" ", $op, " ")
            };
            (@space un, $op:literal) => {
                $op
            };
        }
        macro_rules! group_if {
            ($lvl:expr, $body:block) => {{
                let group = level > $lvl;
                self.p_if(group, "(")?;
                let ret = $body;
                self.p_if(group, ")")?;
                ret
            }};
        }
        match &ast {
            Expr::BoolOp(ruff::ExprBoolOp {
                op,
                values,
                range: _range,
            }) => {
                let (op, prec) = op_prec!(bin, op, BoolOp, And("and", AND), Or("or", OR));
                group_if!(prec, {
                    let mut first = true;
                    for val in values {
                        self.p_delim(&mut first, op)?;
                        self.unparse_expr(val, prec + 1)?;
                    }
                })
            }
            Expr::Named(ruff::ExprNamed {
                target,
                value,
                range: _range,
            }) => {
                group_if!(precedence::TUPLE, {
                    self.unparse_expr(target, precedence::ATOM)?;
                    self.p(" := ")?;
                    self.unparse_expr(value, precedence::ATOM)?;
                })
            }
            Expr::BinOp(ruff::ExprBinOp {
                left,
                op,
                right,
                range: _range,
            }) => {
                let right_associative = matches!(op, Operator::Pow);
                let (op, prec) = op_prec!(
                    bin,
                    op,
                    Operator,
                    Add("+", ARITH),
                    Sub("-", ARITH),
                    Mult("*", TERM),
                    MatMult("@", TERM),
                    Div("/", TERM),
                    Mod("%", TERM),
                    Pow("**", POWER),
                    LShift("<<", SHIFT),
                    RShift(">>", SHIFT),
                    BitOr("|", BOR),
                    BitXor("^", BXOR),
                    BitAnd("&", BAND),
                    FloorDiv("//", TERM),
                );
                group_if!(prec, {
                    self.unparse_expr(left, prec + right_associative as u8)?;
                    self.p(op)?;
                    self.unparse_expr(right, prec + !right_associative as u8)?;
                })
            }
            Expr::UnaryOp(ruff::ExprUnaryOp {
                op,
                operand,
                range: _range,
            }) => {
                let (op, prec) = op_prec!(
                    un,
                    op,
                    ruff::UnaryOp,
                    Invert("~", FACTOR),
                    Not("not ", NOT),
                    UAdd("+", FACTOR),
                    USub("-", FACTOR)
                );
                group_if!(prec, {
                    self.p(op)?;
                    self.unparse_expr(operand, prec)?;
                })
            }
            Expr::Lambda(ruff::ExprLambda {
                parameters,
                body,
                range: _range,
            }) => {
                group_if!(precedence::TEST, {
                    if let Some(parameters) = parameters {
                        self.p("lambda ")?;
                        self.unparse_arguments(parameters)?;
                    } else {
                        self.p("lambda")?;
                    }
                    write!(self, ": {}", unparse_expr(body, self.source))?;
                })
            }
            Expr::If(ruff::ExprIf {
                test,
                body,
                orelse,
                range: _range,
            }) => {
                group_if!(precedence::TEST, {
                    self.unparse_expr(body, precedence::TEST + 1)?;
                    self.p(" if ")?;
                    self.unparse_expr(test, precedence::TEST + 1)?;
                    self.p(" else ")?;
                    self.unparse_expr(orelse, precedence::TEST)?;
                })
            }
            Expr::Dict(ruff::ExprDict {
                items,
                range: _range,
            }) => {
                self.p("{")?;
                let mut first = true;
                for item in items {
                    self.p_delim(&mut first, ", ")?;
                    if let Some(k) = &item.key {
                        write!(self, "{}: ", unparse_expr(k, self.source))?;
                    } else {
                        self.p("**")?;
                    }
                    self.unparse_expr(&item.value, level)?;
                }
                self.p("}")?;
            }
            Expr::Set(ruff::ExprSet {
                elts,
                range: _range,
            }) => {
                self.p("{")?;
                let mut first = true;
                for v in elts {
                    self.p_delim(&mut first, ", ")?;
                    self.unparse_expr(v, precedence::TEST)?;
                }
                self.p("}")?;
            }
            Expr::ListComp(ruff::ExprListComp {
                elt,
                generators,
                range: _range,
            }) => {
                self.p("[")?;
                self.unparse_expr(elt, precedence::TEST)?;
                self.unparse_comp(generators)?;
                self.p("]")?;
            }
            Expr::SetComp(ruff::ExprSetComp {
                elt,
                generators,
                range: _range,
            }) => {
                self.p("{")?;
                self.unparse_expr(elt, precedence::TEST)?;
                self.unparse_comp(generators)?;
                self.p("}")?;
            }
            Expr::DictComp(ruff::ExprDictComp {
                key,
                value,
                generators,
                range: _range,
            }) => {
                self.p("{")?;
                self.unparse_expr(key, precedence::TEST)?;
                self.p(": ")?;
                self.unparse_expr(value, precedence::TEST)?;
                self.unparse_comp(generators)?;
                self.p("}")?;
            }
            Expr::Generator(ruff::ExprGenerator {
                parenthesized: _,
                elt,
                generators,
                range: _range,
            }) => {
                self.p("(")?;
                self.unparse_expr(elt, precedence::TEST)?;
                self.unparse_comp(generators)?;
                self.p(")")?;
            }
            Expr::Await(ruff::ExprAwait {
                value,
                range: _range,
            }) => {
                group_if!(precedence::AWAIT, {
                    self.p("await ")?;
                    self.unparse_expr(value, precedence::ATOM)?;
                })
            }
            Expr::Yield(ruff::ExprYield {
                value,
                range: _range,
            }) => {
                if let Some(value) = value {
                    write!(self, "(yield {})", unparse_expr(value, self.source))?;
                } else {
                    self.p("(yield)")?;
                }
            }
            Expr::YieldFrom(ruff::ExprYieldFrom {
                value,
                range: _range,
            }) => {
                write!(self, "(yield from {})", unparse_expr(value, self.source))?;
            }
            Expr::Compare(ruff::ExprCompare {
                left,
                ops,
                comparators,
                range: _range,
            }) => {
                group_if!(precedence::CMP, {
                    let new_lvl = precedence::CMP + 1;
                    self.unparse_expr(left, new_lvl)?;
                    for (op, cmp) in ops.iter().zip(comparators) {
                        self.p(" ")?;
                        self.p(op.as_str())?;
                        self.p(" ")?;
                        self.unparse_expr(cmp, new_lvl)?;
                    }
                })
            }
            Expr::Call(ruff::ExprCall {
                func,
                arguments: Arguments { args, keywords, .. },
                range: _range,
            }) => {
                self.unparse_expr(func, precedence::ATOM)?;
                self.p("(")?;
                if let (
                    [
                        Expr::Generator(ruff::ExprGenerator {
                            elt,
                            generators,
                            range: _range,
                            ..
                        }),
                    ],
                    [],
                ) = (&**args, &**keywords)
                {
                    // make sure a single genexpr doesn't get double parens
                    self.unparse_expr(elt, precedence::TEST)?;
                    self.unparse_comp(generators)?;
                } else {
                    let mut first = true;
                    for arg in args {
                        self.p_delim(&mut first, ", ")?;
                        self.unparse_expr(arg, precedence::TEST)?;
                    }
                    for kw in keywords {
                        self.p_delim(&mut first, ", ")?;
                        if let Some(arg) = &kw.arg {
                            self.p_id(arg)?;
                            self.p("=")?;
                        } else {
                            self.p("**")?;
                        }
                        self.unparse_expr(&kw.value, precedence::TEST)?;
                    }
                }
                self.p(")")?;
            }
            Expr::FString(ruff::ExprFString { value, .. }) => self.unparse_fstring(value)?,
            Expr::StringLiteral(ruff::ExprStringLiteral { value, .. }) => {
                if value.is_unicode() {
                    self.p("u")?
                }
                UnicodeEscape::new_repr(value.to_str().as_ref())
                    .str_repr()
                    .fmt(self.f)?
            }
            Expr::BytesLiteral(ruff::ExprBytesLiteral { value, .. }) => {
                AsciiEscape::new_repr(&value.bytes().collect::<Vec<_>>())
                    .bytes_repr()
                    .fmt(self.f)?
            }
            Expr::NumberLiteral(ruff::ExprNumberLiteral { value, .. }) => {
                const { assert!(f64::MAX_10_EXP == 308) };
                let inf_str = "1e309";
                match value {
                    ruff::Number::Int(int) => int.fmt(self.f)?,
                    &ruff::Number::Float(fp) => {
                        if fp.is_infinite() {
                            self.p(inf_str)?
                        } else {
                            self.p(&rustpython_literal::float::to_string(fp))?
                        }
                    }
                    &ruff::Number::Complex { real, imag } => self
                        .p(&rustpython_literal::complex::to_string(real, imag)
                            .replace("inf", inf_str))?,
                }
            }
            Expr::BooleanLiteral(ruff::ExprBooleanLiteral { value, .. }) => {
                self.p(if *value { "True" } else { "False" })?
            }
            Expr::NoneLiteral(ruff::ExprNoneLiteral { .. }) => self.p("None")?,
            Expr::EllipsisLiteral(ruff::ExprEllipsisLiteral { .. }) => self.p("...")?,
            Expr::Attribute(ruff::ExprAttribute { value, attr, .. }) => {
                self.unparse_expr(value, precedence::ATOM)?;
                let period = if let Expr::NumberLiteral(ruff::ExprNumberLiteral {
                    value: ruff::Number::Int(_),
                    ..
                }) = value.as_ref()
                {
                    " ."
                } else {
                    "."
                };
                self.p(period)?;
                self.p_id(attr)?;
            }
            Expr::Subscript(ruff::ExprSubscript { value, slice, .. }) => {
                self.unparse_expr(value, precedence::ATOM)?;
                let lvl = precedence::TUPLE;
                self.p("[")?;
                self.unparse_expr(slice, lvl)?;
                self.p("]")?;
            }
            Expr::Starred(ruff::ExprStarred { value, .. }) => {
                self.p("*")?;
                self.unparse_expr(value, precedence::EXPR)?;
            }
            Expr::Name(ruff::ExprName { id, .. }) => self.p(id.as_str())?,
            Expr::List(ruff::ExprList { elts, .. }) => {
                self.p("[")?;
                let mut first = true;
                for elt in elts {
                    self.p_delim(&mut first, ", ")?;
                    self.unparse_expr(elt, precedence::TEST)?;
                }
                self.p("]")?;
            }
            Expr::Tuple(ruff::ExprTuple { elts, .. }) => {
                if elts.is_empty() {
                    self.p("()")?;
                } else {
                    group_if!(precedence::TUPLE, {
                        let mut first = true;
                        for elt in elts {
                            self.p_delim(&mut first, ", ")?;
                            self.unparse_expr(elt, precedence::TEST)?;
                        }
                        self.p_if(elts.len() == 1, ",")?;
                    })
                }
            }
            Expr::Slice(ruff::ExprSlice {
                lower,
                upper,
                step,
                range: _range,
            }) => {
                if let Some(lower) = lower {
                    self.unparse_expr(lower, precedence::TEST)?;
                }
                self.p(":")?;
                if let Some(upper) = upper {
                    self.unparse_expr(upper, precedence::TEST)?;
                }
                if let Some(step) = step {
                    self.p(":")?;
                    self.unparse_expr(step, precedence::TEST)?;
                }
            }
            Expr::IpyEscapeCommand(_) => {}
        }
        Ok(())
    }

    fn unparse_arguments(&mut self, args: &Parameters) -> fmt::Result {
        let mut first = true;
        for (i, arg) in args.posonlyargs.iter().chain(&args.args).enumerate() {
            self.p_delim(&mut first, ", ")?;
            self.unparse_function_arg(arg)?;
            self.p_if(i + 1 == args.posonlyargs.len(), ", /")?;
        }
        if args.vararg.is_some() || !args.kwonlyargs.is_empty() {
            self.p_delim(&mut first, ", ")?;
            self.p("*")?;
        }
        if let Some(vararg) = &args.vararg {
            self.unparse_arg(vararg)?;
        }
        for kwarg in args.kwonlyargs.iter() {
            self.p_delim(&mut first, ", ")?;
            self.unparse_function_arg(kwarg)?;
        }
        if let Some(kwarg) = &args.kwarg {
            self.p_delim(&mut first, ", ")?;
            self.p("**")?;
            self.unparse_arg(kwarg)?;
        }
        Ok(())
    }
    fn unparse_function_arg(&mut self, arg: &ParameterWithDefault) -> fmt::Result {
        self.unparse_arg(&arg.parameter)?;
        if let Some(default) = &arg.default {
            write!(self, "={}", unparse_expr(default, self.source))?;
        }
        Ok(())
    }

    fn unparse_arg(&mut self, arg: &Parameter) -> fmt::Result {
        self.p_id(&arg.name)?;
        if let Some(ann) = &arg.annotation {
            write!(self, ": {}", unparse_expr(ann, self.source))?;
        }
        Ok(())
    }

    fn unparse_comp(&mut self, generators: &[Comprehension]) -> fmt::Result {
        for comp in generators {
            self.p(if comp.is_async {
                " async for "
            } else {
                " for "
            })?;
            self.unparse_expr(&comp.target, precedence::TUPLE)?;
            self.p(" in ")?;
            self.unparse_expr(&comp.iter, precedence::TEST + 1)?;
            for cond in &comp.ifs {
                self.p(" if ")?;
                self.unparse_expr(cond, precedence::TEST + 1)?;
            }
        }
        Ok(())
    }

    fn unparse_fstring_body(&mut self, elements: &[ruff::FStringElement]) -> fmt::Result {
        for elem in elements {
            self.unparse_fstring_elem(elem)?;
        }
        Ok(())
    }

    fn unparse_formatted(
        &mut self,
        val: &Expr,
        debug_text: Option<&ruff::DebugText>,
        conversion: ConversionFlag,
        spec: Option<&ruff::FStringFormatSpec>,
    ) -> fmt::Result {
        let buffered = to_string_fmt(|f| {
            Unparser::new(f, self.source).unparse_expr(val, precedence::TEST + 1)
        });
        if let Some(ruff::DebugText { leading, trailing }) = debug_text {
            self.p(leading)?;
            self.p(self.source.slice(val.range()))?;
            self.p(trailing)?;
        }
        let brace = if buffered.starts_with('{') {
            // put a space to avoid escaping the bracket
            "{ "
        } else {
            "{"
        };
        self.p(brace)?;
        self.p(&buffered)?;
        drop(buffered);

        if conversion != ConversionFlag::None {
            self.p("!")?;
            let buf = &[conversion as u8];
            let c = std::str::from_utf8(buf).unwrap();
            self.p(c)?;
        }

        if let Some(spec) = spec {
            self.p(":")?;
            self.unparse_fstring_body(&spec.elements)?;
        }

        self.p("}")?;

        Ok(())
    }

    fn unparse_fstring_elem(&mut self, elem: &ruff::FStringElement) -> fmt::Result {
        match elem {
            ruff::FStringElement::Expression(ruff::FStringExpressionElement {
                expression,
                debug_text,
                conversion,
                format_spec,
                ..
            }) => self.unparse_formatted(
                expression,
                debug_text.as_ref(),
                *conversion,
                format_spec.as_deref(),
            ),
            ruff::FStringElement::Literal(ruff::FStringLiteralElement { value, .. }) => {
                self.unparse_fstring_str(value)
            }
        }
    }

    fn unparse_fstring_str(&mut self, s: &str) -> fmt::Result {
        let s = s.replace('{', "{{").replace('}', "}}");
        self.p(&s)
    }

    fn unparse_fstring(&mut self, value: &ruff::FStringValue) -> fmt::Result {
        self.p("f")?;
        let body = to_string_fmt(|f| {
            value.iter().try_for_each(|part| match part {
                ruff::FStringPart::Literal(lit) => f.write_str(lit),
                ruff::FStringPart::FString(ruff::FString { elements, .. }) => {
                    Unparser::new(f, self.source).unparse_fstring_body(elements)
                }
            })
        });
        // .unparse_fstring_body(elements));
        UnicodeEscape::new_repr(body.as_str().as_ref())
            .str_repr()
            .write(self.f)
    }
}

pub struct UnparseExpr<'a> {
    expr: &'a Expr,
    source: &'a SourceFile,
}

pub const fn unparse_expr<'a>(expr: &'a Expr, source: &'a SourceFile) -> UnparseExpr<'a> {
    UnparseExpr { expr, source }
}

impl fmt::Display for UnparseExpr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Unparser::new(f, self.source).unparse_expr(self.expr, precedence::TEST)
    }
}

fn to_string_fmt(f: impl FnOnce(&mut fmt::Formatter<'_>) -> fmt::Result) -> String {
    use std::cell::Cell;
    struct Fmt<F>(Cell<Option<F>>);
    impl<F: FnOnce(&mut fmt::Formatter<'_>) -> fmt::Result> fmt::Display for Fmt<F> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.take().unwrap()(f)
        }
    }
    Fmt(Cell::new(Some(f))).to_string()
}
