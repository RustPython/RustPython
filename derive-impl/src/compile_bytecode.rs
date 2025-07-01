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
use crate::util::{check_duplicate, check_duplicate_msg};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use rustpython_compiler_core::{Mode, bytecode::CodeObject, frozen};
use std::sync::LazyLock;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};
use syn::Token;
use syn::{
    self, LitByteStr, LitStr, Macro,
    parse::{ParseStream, Parser, Result as ParseResult},
    punctuated::Punctuated,
    spanned::Spanned,
};

static CARGO_MANIFEST_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
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
    /// Direct Raw source code
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
            par_map(dir.modules, do_compile).collect()
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

fn par_map<T, U, I, F>(it: I, f: F) -> impl Iterator<Item = U>
where
    I: IntoIterator<Item = T, IntoIter: ExactSizeIterator + Send>,
    F: Fn(T) -> U + Sync,
    U: Send,
{
    let it = it.into_iter();
    let mut out = Vec::from_iter(std::iter::repeat_with(|| None).take(it.len()));
    let it = std::sync::Mutex::new(std::iter::zip(&mut out, it));
    let task = || {
        while let Some((out, x)) = { it.lock().unwrap().next() } {
            *out = Some(f(x));
        }
    };
    std::thread::scope(|s| {
        let nproc = std::thread::available_parallelism().unwrap().get();
        for _ in 0..nproc {
            std::thread::Builder::new()
                .stack_size(4 * 1024 * 1024)
                .spawn_scoped(s, task)
                .unwrap();
        }
    });
    out.into_iter().map(Option::unwrap)
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

impl PyCompileArgs {
    fn parse(input: TokenStream, allow_lib: bool) -> Result<PyCompileArgs, Diagnostic> {
        let mut module_name = None;
        let mut mode = None;
        let mut source: Option<CompilationSource> = None;
        let mut source_span = (Span::call_site(), Span::call_site());
        let mut crate_name = None;
        let mut exclude = None;

        syn::meta::parser(|meta| {
            let assert_source_empty =
                || check_duplicate_msg(&meta, &source, "Cannot have more than one source");

            let ident = meta
                .path
                .get_ident()
                .ok_or_else(|| meta.error("unknown arg"))?;
            let check_str = || meta.value()?.call(parse_str);
            if ident == "mode" {
                check_duplicate(&meta, &mode)?;
                let s = check_str()?;
                match s.value().parse() {
                    Ok(mode_val) => mode = Some(mode_val),
                    Err(e) => bail_span!(s, "{}", e),
                }
            } else if ident == "module_name" {
                check_duplicate(&meta, &module_name)?;
                module_name = Some(check_str()?.value())
            } else if ident == "source" {
                assert_source_empty()?;
                let code = check_str()?.value();
                source_span = (ident.span(), code.span());
                source = Some(CompilationSource::SourceCode(code));
            } else if ident == "path" {
                assert_source_empty()?;
                let path = check_str()?;
                source_span = (ident.span(), path.span());
                source = Some(CompilationSource::Path(path.value().into()));
            } else if ident == "lib_path" {
                if !allow_lib {
                    bail_span!(ident, "py_compile doesn't accept lib_path")
                }

                assert_source_empty()?;
                let path = check_str()?;
                source_span = (ident.span(), path.span());
                source = Some(CompilationSource::LibPath(path.value().into()));
            } else if ident == "crate_name" {
                check_duplicate(&meta, &crate_name)?;
                let name = check_str()?.parse()?;
                crate_name = Some(name);
            } else if ident == "exclude" {
                check_duplicate(&meta, &exclude)?;
                let input = meta.value()?;
                let content;
                syn::bracketed!(content in input);
                exclude = Some(Punctuated::parse_terminated(&content)?);
            } else {
                return Err(meta.error("unknown attr"));
            }
            Ok(())
        })
        .parse2(input)?;

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
            crate_name: crate_name.unwrap_or_else(|| syn::parse_quote!(::rustpython_vm)),
            exclude: exclude.unwrap_or_default(),
        })
    }
}

fn parse_str(input: ParseStream<'_>) -> ParseResult<LitStr> {
    let span = input.span();
    if input.peek(LitStr) {
        input.parse()
    } else if let Ok(mac) = input.parse::<Macro>() {
        Ok(LitStr::new(&mac.tokens.to_string(), mac.span()))
    } else {
        Err(syn::Error::new(span, "Expected string or stringify macro"))
    }
}

struct PyCompileArgs {
    source: CompilationSource,
    source_span: (Span, Span),
    mode: Mode,
    module_name: String,
    crate_name: syn::Path,
    exclude: Punctuated<LitStr, Token![,]>,
}

pub fn impl_py_compile(
    input: TokenStream,
    compiler: &dyn Compiler,
) -> Result<TokenStream, Diagnostic> {
    let args = PyCompileArgs::parse(input, false)?;

    let crate_name = args.crate_name;
    let code = args
        .source
        .compile_single(args.mode, &args.module_name, compiler)
        .map_err(|msg| Diagnostic::spans_error(args.source_span, msg))?;

    let frozen = frozen::FrozenCodeObject::encode(&code);
    let bytes = LitByteStr::new(&frozen.bytes, Span::call_site());

    let output = quote! {
        #crate_name::frozen::FrozenCodeObject { bytes: &#bytes[..] }
    };

    Ok(output)
}

pub fn impl_py_freeze(
    input: TokenStream,
    compiler: &dyn Compiler,
) -> Result<TokenStream, Diagnostic> {
    let args = PyCompileArgs::parse(input, true)?;

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

    let data = frozen::FrozenLib::encode(code_map.iter().map(|(k, v)| {
        let v = frozen::FrozenModule {
            code: frozen::FrozenCodeObject::encode(&v.code),
            package: v.package,
        };
        (&**k, v)
    }));
    let bytes = LitByteStr::new(&data.bytes, Span::call_site());

    let output = quote! {
        #crate_name::frozen::FrozenLib::from_ref(#bytes)
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
            mut path: std::str::Chars<'_>,
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
                                    return MatchResult::SubPatternDoesntMatch;
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
                        let Some(c) = path.next() else {
                            return MatchResult::EntirePatternDoesntMatch;
                        };
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

        let pattern: ModulePattern = "foo.**.bar*".parse().unwrap();
        assert!(pattern.matches("foo.quuxxx.barbaz"));
        assert!(pattern.matches("foo.quux.asdf.barp"));
        assert!(!pattern.matches("asdf.foo.barbaz"));
    }
}
