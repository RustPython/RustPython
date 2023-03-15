//! Parsing and processing for this form:
//! ```ignore
//! py_compile!(
//!     // either:
//!     source = "python_source_code",
//!     // or
//!     file = "file/path/relative/to/$CARGO_MANIFEST_DIR",
//!
//!     // the mode to compile the code in
//!     mode = "exec", // or "eval" or "single"
//!     // the path put into the CodeObject, defaults to "frozen"
//!     module_name = "frozen",
//! )
//! ```

use crate::Diagnostic;
use once_cell::sync::Lazy;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use rustpython_compiler_core::{frozen_lib, CodeObject, Mode};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};
use syn::{
    self,
    parse::{Parse, ParseStream, Result as ParseResult},
    parse2, LitByteStr, LitStr, Token,
};

static CARGO_MANIFEST_DIR: Lazy<PathBuf> = Lazy::new(|| {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not present"))
});
fn resolve_path(path: &Path) -> std::borrow::Cow<'_, Path> {
    if path.is_absolute() {
        path.into()
    } else {
        CARGO_MANIFEST_DIR.join(path).into()
    }
}

enum CompilationSource {
    /// Source is a single module
    Path(PathBuf),
    /// Direct Raw sourcecode
    SourceCode(String),
    /// Source is a directory of modules
    LibPath(PathBuf),
}

#[derive(Clone)]
struct CompiledModule {
    code: CodeObject,
    package: bool,
}

pub trait Compiler: Sync {
    fn compile(
        &self,
        source: &str,
        mode: Mode,
        source_path: String,
    ) -> Result<CodeObject, Box<dyn std::error::Error>>;
}

impl CompilationSource {
    fn compile_string(
        source: &str,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
    ) -> Result<CodeObject, Box<dyn std::error::Error>> {
        compiler.compile(source, mode, format!("<frozen {module_name}>"))
    }

    fn compile(
        &self,
        mode: Mode,
        module_name: String,
        excludes: &[pattern::ModulePattern],
        compiler: &dyn Compiler,
    ) -> Result<Vec<(String, CompiledModule)>, String> {
        let mut dir = DirWalker::new(excludes);
        match self {
            CompilationSource::LibPath(path) => dir.walk(&resolve_path(path), "")?,
            CompilationSource::Path(path) => dir.add_entry(resolve_path(path).into(), "")?,
            CompilationSource::SourceCode(_) => {
                let module = CompiledModule {
                    code: self.compile_single(mode, &module_name, compiler)?,
                    package: false,
                };
                return Ok(vec![(module_name, module)]);
            }
        }
        let do_compile = |(module_name, (path, package)): (String, (PathBuf, _))| {
            let code = Self::compile_file(&path, mode, &module_name, compiler)?;
            Ok((module_name, CompiledModule { code, package }))
        };
        if dir.modules.len() > 32 {
            let nmodules = dir.modules.len();
            let modules = std::sync::Mutex::new(dir.modules.into_iter().enumerate());
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::scope(|s| {
                let nproc = std::thread::available_parallelism().unwrap().get();
                for tx in itertools::repeat_n(tx, nproc) {
                    let modules = &modules;
                    std::thread::Builder::new()
                        .stack_size(4 * 1024 * 1024)
                        .spawn_scoped(s, move || loop {
                            let Some((i, module)) = modules.lock().unwrap().next() else { return };
                            tx.send((i, do_compile(module))).unwrap();
                        })
                        .unwrap();
                }
            });
            let mut out = vec![None; nmodules];
            for (i, module) in rx {
                out[i] = Some(module);
            }
            out.into_iter().map(Option::unwrap).collect()
        } else {
            dir.modules.into_iter().map(do_compile).collect()
        }
    }

    fn compile_file(
        path: &Path,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
    ) -> Result<CodeObject, String> {
        let compile_path = |src_path: &Path| {
            let source = fs::read_to_string(resolve_path(src_path))
                .map_err(|err| format!("Error reading file {path:?}: {err}"))?;
            Self::compile_string(&source, mode, module_name, compiler).map_err(|err| {
                let rel_path = path.strip_prefix(&*CARGO_MANIFEST_DIR).unwrap_or(path);
                format!("Python compile error in {}: {err}", rel_path.display())
            })
        };
        compile_path(path).or_else(|e| {
            if cfg!(windows) {
                if let Ok(real_path) = fs::read_to_string(path.canonicalize().unwrap()) {
                    let joined = path.parent().unwrap().join(real_path.trim());
                    if joined.exists() {
                        return compile_path(&joined);
                    } else {
                        return Err(e);
                    }
                }
            }
            Err(e)
        })
    }

    fn compile_single(
        &self,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
    ) -> Result<CodeObject, String> {
        match self {
            CompilationSource::Path(path) => Self::compile_file(path, mode, module_name, compiler),
            CompilationSource::SourceCode(code) => {
                Self::compile_string(&textwrap::dedent(code), mode, module_name, compiler)
                    .map_err(|err| format!("Python compile error in string literal: {err}"))
            }
            CompilationSource::LibPath(_) => {
                unreachable!("Can't use compile_single with lib source")
            }
        }
    }
}

#[derive(Default)]
struct DirWalker<'a> {
    excludes: &'a [pattern::ModulePattern],
    modules: BTreeMap<String, (PathBuf, bool)>,
}

impl<'a> DirWalker<'a> {
    fn new(excludes: &'a [pattern::ModulePattern]) -> Self {
        Self {
            excludes,
            modules: BTreeMap::new(),
        }
    }
    fn walk(&mut self, path: &Path, parent: &str) -> Result<(), String> {
        let paths = fs::read_dir(path)
            .or_else(|e| {
                if cfg!(windows) {
                    if let Ok(real_path) = fs::read_to_string(path.canonicalize().unwrap()) {
                        return fs::read_dir(real_path.trim());
                    }
                }
                Err(e)
            })
            .map_err(|err| format!("Error listing dir {path:?}: {err}"))?;
        for path in paths {
            let path = path.map_err(|err| format!("Failed to list file: {err}"))?;
            self.add_entry(path.path(), parent)?;
        }
        Ok(())
    }
    fn add_entry(&mut self, path: PathBuf, parent: &str) -> Result<(), String> {
        let file_name = path
            .file_name()
            .unwrap()
            .to_str()
            .ok_or_else(|| format!("Invalid UTF-8 in file name {path:?}"))?;
        if path.is_dir() {
            if parent.is_empty() {
                self.walk(&path, file_name)?
            } else {
                self.walk(&path, &[parent, ".", file_name].concat())?
            }
        } else if file_name.ends_with(".py") {
            let stem = path.file_stem().unwrap().to_str().unwrap();
            let is_init = stem == "__init__";
            let module_name = if is_init {
                parent.to_owned()
            } else if parent.is_empty() {
                stem.to_owned()
            } else {
                [parent, ".", stem].concat()
            };

            if !self.excludes.iter().any(|pat| pat.matches(&module_name)) {
                self.modules.insert(module_name, (path, is_init));
            }
        }
        Ok(())
    }
}

mod kw {
    syn::custom_keyword!(stringify);
    syn::custom_keyword!(mode);
    syn::custom_keyword!(module_name);
    syn::custom_keyword!(source);
    syn::custom_keyword!(path);
    syn::custom_keyword!(lib_path);
    syn::custom_keyword!(crate_name);
    syn::custom_keyword!(exclude);
}

fn check_duplicate<T>(x: &Option<T>, span: Span) -> syn::Result<()> {
    if x.is_none() {
        Ok(())
    } else {
        Err(syn::Error::new(span, "duplicate option"))
    }
}

impl Parse for PyCompileArgs {
    fn parse(input: ParseStream) -> ParseResult<Self> {
        let mut module_name = None;
        let mut mode = None;
        let mut source: Option<CompilationSource> = None;
        let mut source_span = (Span::call_site(), Span::call_site());
        let mut crate_name = None;
        let mut exclude = None;

        loop {
            if input.is_empty() {
                break;
            }
            match_tok!(match input {
                tok @ kw::mode => {
                    check_duplicate(&mode, tok.span)?;
                    input.parse::<Token![=]>()?;
                    let s = input.call(parse_litstr)?;
                    let mode_val = s
                        .value()
                        .parse()
                        .map_err(|e| syn::Error::new(s.span(), e))?;
                    mode = Some(mode_val);
                }
                tok @ kw::module_name => {
                    check_duplicate(&module_name, tok.span)?;
                    input.parse::<Token![=]>()?;
                    module_name = Some(input.call(parse_litstr)?.value())
                }
                tok @ kw::source => {
                    check_duplicate(&source, tok.span)?;
                    input.parse::<Token![=]>()?;
                    let code = input.call(parse_litstr)?;
                    source = Some(CompilationSource::SourceCode(code.value()));
                    source_span = (tok.span, code.span());
                }
                tok @ kw::path => {
                    check_duplicate(&source, tok.span)?;
                    input.parse::<Token![=]>()?;
                    let path = input.call(parse_litstr)?;
                    source = Some(CompilationSource::Path(path.value().into()));
                    source_span = (tok.span, path.span());
                }
                tok @ kw::lib_path => {
                    check_duplicate(&source, tok.span)?;
                    input.parse::<Token![=]>()?;
                    let path = input.call(parse_litstr)?;
                    source = Some(CompilationSource::LibPath(path.value().into()));
                    source_span = (tok.span, path.span());
                }
                tok @ kw::crate_name => {
                    check_duplicate(&crate_name, tok.span)?;
                    input.parse::<Token![=]>()?;
                    crate_name = Some(input.call(parse_litstr)?.parse()?);
                }
                tok @ kw::exclude => {
                    check_duplicate(&exclude, tok.span)?;
                    input.parse::<Token![=]>()?;
                    let content;
                    syn::bracketed!(content in input);
                    exclude = Some(content.parse_terminated(parse_litstr)?);
                }
            });

            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }

        let source = source.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "Must have either file or source in py_compile!()/py_freeze!()",
            )
        })?;

        Ok(PyCompileArgs {
            source,
            source_span,
            mode: mode.unwrap_or(Mode::Exec),
            module_name: module_name.unwrap_or_else(|| "frozen".to_owned()),
            crate_name: crate_name.unwrap_or_else(|| syn::parse_quote!(::rustpython_vm::bytecode)),
            exclude: exclude.unwrap_or_default(),
        })
    }
}

fn parse_litstr(input: ParseStream) -> ParseResult<LitStr> {
    if input.peek(LitStr) {
        input.parse()
    } else if input.peek(kw::stringify) {
        input.parse::<kw::stringify>()?;
        input.parse::<Token![!]>()?;
        let stringify_arg = input.step(|cursor| {
            if let Some((proc_macro2::TokenTree::Group(g), next)) = cursor.token_tree() {
                if g.delimiter() != proc_macro2::Delimiter::None {
                    return Ok((g, next));
                }
            }
            Err(cursor.error("expected delimiter"))
        })?;
        Ok(LitStr::new(
            &stringify_arg.stream().to_string(),
            stringify_arg.span(),
        ))
    } else {
        Err(input.error("expected string literal or stringify macro"))
    }
}

struct PyCompileArgs {
    source: CompilationSource,
    source_span: (Span, Span),
    mode: Mode,
    module_name: String,
    crate_name: syn::Path,
    exclude: syn::punctuated::Punctuated<LitStr, Token![,]>,
}

pub fn impl_py_compile(
    input: TokenStream,
    compiler: &dyn Compiler,
) -> Result<TokenStream, Diagnostic> {
    let args: PyCompileArgs = parse2(input)?;

    if matches!(args.source, CompilationSource::LibPath(_)) {
        return Err(Diagnostic::spans_error(
            args.source_span,
            "py_compile doesn't accept lib",
        ));
    }

    let crate_name = args.crate_name;
    let code = args
        .source
        .compile_single(args.mode, &args.module_name, compiler)
        .map_err(|msg| Diagnostic::spans_error(args.source_span, msg))?;

    let frozen = frozen_lib::FrozenCodeObject::encode(&code);
    let bytes = LitByteStr::new(&frozen.bytes, Span::call_site());

    let output = quote! {
        #crate_name::frozen_lib::FrozenCodeObject { bytes: &#bytes[..] }
    };

    Ok(output)
}

pub fn impl_py_freeze(
    input: TokenStream,
    compiler: &dyn Compiler,
) -> Result<TokenStream, Diagnostic> {
    let args: PyCompileArgs = parse2(input)?;

    let excludes = args
        .exclude
        .into_iter()
        .map(|s| s.value().parse().map_err(|e| syn::Error::new(s.span(), e)))
        .collect::<Result<Vec<_>, _>>()?;

    let crate_name = args.crate_name;
    let code_map = args
        .source
        .compile(args.mode, args.module_name, &excludes, compiler)
        .map_err(|msg| Diagnostic::spans_error(args.source_span, msg))?;

    let data = frozen_lib::FrozenLib::encode(code_map.iter().map(|(k, v)| {
        let v = frozen_lib::FrozenModule {
            code: frozen_lib::FrozenCodeObject::encode(&v.code),
            package: v.package,
        };
        (&**k, v)
    }));
    let bytes = LitByteStr::new(&data.bytes, Span::call_site());

    let output = quote! {
        #crate_name::frozen_lib::FrozenLib::from_ref(#bytes)
    };

    Ok(output)
}

mod pattern {
    pub struct ModulePattern {
        tokens: Vec<Token>,
    }

    #[derive(Copy, Clone, Debug)]
    enum Token {
        DoubleStar,
        Star,
        Char(char),
    }

    #[derive(Debug)]
    pub enum PatternError {
        BadDoubleStar,
    }
    impl std::fmt::Display for PatternError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                PatternError::BadDoubleStar => {
                    f.write_str("`**` must be alone in a path component")
                }
            }
        }
    }

    impl std::str::FromStr for ModulePattern {
        type Err = PatternError;
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let mut chars = s.chars().peekable();
            let mut was_dot = true;
            let tokens = std::iter::from_fn(|| {
                chars.next().map(|c| match c {
                    '*' if chars.peek() == Some(&'*') => {
                        chars.next();
                        if was_dot && matches!(chars.next(), None | Some('.')) {
                            Ok(Token::DoubleStar)
                        } else {
                            Err(PatternError::BadDoubleStar)
                        }
                    }
                    '*' => Ok(Token::Star),
                    c => {
                        was_dot = c == '.';
                        Ok(Token::Char(c))
                    }
                })
            });
            let tokens = tokens.collect::<Result<_, _>>()?;
            Ok(Self { tokens })
        }
    }

    impl ModulePattern {
        pub fn matches(&self, s: &str) -> bool {
            self.matches_from(true, s.chars(), 0) == MatchResult::Match
        }
        // vaguely based off glob's matches_from
        fn matches_from(
            &self,
            mut follows_separator: bool,
            mut path: std::str::Chars,
            i: usize,
        ) -> MatchResult {
            for (ti, &token) in self.tokens[i..].iter().enumerate() {
                match token {
                    Token::Star | Token::DoubleStar => {
                        // Empty match
                        match self.matches_from(follows_separator, path.clone(), i + ti + 1) {
                            MatchResult::SubPatternDoesntMatch => {} // keep trying
                            m => return m,
                        }

                        while let Some(c) = path.next() {
                            follows_separator = c == '.';
                            match token {
                                Token::DoubleStar if !follows_separator => continue,
                                Token::Star if follows_separator => {
                                    return MatchResult::SubPatternDoesntMatch
                                }
                                _ => {}
                            }
                            match self.matches_from(follows_separator, path.clone(), i + ti + 1) {
                                MatchResult::SubPatternDoesntMatch => {} // keep trying
                                m => return m,
                            }
                        }
                    }
                    Token::Char(exp) => {
                        let Some(c) = path.next() else { return MatchResult::EntirePatternDoesntMatch };
                        if c != exp {
                            return MatchResult::SubPatternDoesntMatch;
                        }
                        follows_separator = c == '.';
                    }
                }
            }

            // Iter is fused.
            if path.next().is_none() {
                MatchResult::Match
            } else {
                MatchResult::SubPatternDoesntMatch
            }
        }
    }

    #[derive(PartialEq, Eq, Debug)]
    enum MatchResult {
        Match,
        SubPatternDoesntMatch,
        EntirePatternDoesntMatch,
    }

    #[cfg(test)]
    #[test]
    fn test_pattern() {
        let pattern: ModulePattern = "x.bar.foo_*.a".parse().unwrap();
        assert!(pattern.matches("x.bar.foo_asdf.a"));
        assert!(pattern.matches("x.bar.foo_bazzzz.a"));
        assert!(pattern.matches("x.bar.foo_.a"));
        assert!(!pattern.matches("x.bar.foo_"));
        assert!(!pattern.matches("x.bar.foo_quxxx"));
        assert!(!pattern.matches("foo_b.a"));

        let pattern: ModulePattern = "**.foo.**".parse().unwrap();
        assert!(pattern.matches("ba.bazzz.foo.quux"));

        let pattern: ModulePattern = "*.foo.**".parse().unwrap();
        assert!(pattern.matches("ba.foo.baz.quux"));
        assert!(pattern.matches("asdf.foo.barrr"));

        let pattern: ModulePattern = "foo.**".parse().unwrap();
        assert!(pattern.matches("foo.baaar.qx"));
        assert!(!pattern.matches("asdf.foo.brrrr"));
    }
}
