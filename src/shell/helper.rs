#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use rustpython_vm::{
    AsObject, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyDictRef, PyStrRef},
    function::ArgIterable,
    identifier,
};

pub struct ShellHelper<'vm> {
    vm: &'vm VirtualMachine,
    globals: PyDictRef,
}

impl<'vm> ShellHelper<'vm> {
    pub const fn new(vm: &'vm VirtualMachine, globals: PyDictRef) -> Self {
        Self { vm, globals }
    }

    fn get_available_completions<'w>(
        &self,
        words: &'w [String],
    ) -> Option<(&'w str, impl Iterator<Item = PyResult<PyStrRef>> + 'vm)> {
        let (first, rest) = words.split_first()?;

        let get_str_iter = |obj, name| {
            let iter = self.vm.call_special_method(obj, name, ())?;
            ArgIterable::<PyStrRef>::try_from_object(self.vm, iter)?.iter(self.vm)
        };

        if let Some((last, parents)) = rest.split_last() {
            let mut current = self.globals.get_item_opt(first, self.vm).ok()??;
            for attr in parents {
                current = current.get_attr(self.vm.ctx.new_str(attr), self.vm).ok()?;
            }
            let iter = get_str_iter(&current, identifier!(self.vm, __dir__)).ok()?;
            Some((last, iter))
        } else {
            let globals = get_str_iter(self.globals.as_object(), identifier!(self.vm, keys)).ok()?;
            let builtins = get_str_iter(self.vm.builtins.as_object(), identifier!(self.vm, __dir__)).ok()?;
            Some((first, globals.chain(builtins)))
        }
    }

    fn complete_opt(&self, line: &str) -> Option<(usize, Vec<String>)> {
        let (startpos, words) = split_idents_on_dot(line)?;
        let (prefix, completions) = self.get_available_completions(&words)?;

        let completions = completions
            .filter_map(Result::ok)
            .filter(|s| prefix.is_empty() || s.as_str().starts_with(prefix))
            .collect::<Vec<_>>();

        let filtered = if prefix.starts_with('_') {
            completions
        } else {
            let no_underscore: Vec<_> = completions
                .iter()
                .filter(|s| !s.as_str().starts_with('_'))
                .cloned()
                .collect();
            if no_underscore.is_empty() {
                completions
            } else {
                no_underscore
            }
        };

        let mut result = filtered
            .into_iter()
            .map(|s| s.as_str().to_owned())
            .collect::<Vec<_>>();
        result.sort_unstable();

        Some((startpos, result))
    }
}

fn split_idents_on_dot(line: &str) -> Option<(usize, Vec<String>)> {
    let mut idents = vec![String::new()];
    let mut startpos = 0;
    let mut rev_chars = line.chars().rev().enumerate();

    while let Some((i, c)) = rev_chars.next() {
        match c {
            '.' => {
                if idents.last().map_or(false, |s| s.is_empty()) && i != 0 {
                    return None; // Double dot
                }
                idents.last_mut().map(|s| reverse_string(s));
                if idents.len() == 1 {
                    startpos = line.len() - i;
                }
                idents.push(String::new());
            }
            c if c.is_alphanumeric() || c == '_' => {
                idents.last_mut()?.push(c);
            }
            _ => {
                if idents.len() == 1 && idents.last().map_or(true, |s| s.is_empty()) {
                    return None;
                }
                startpos = line.len() - i;
                break;
            }
        }
    }

    if idents == [String::new()] {
        return None;
    }

    idents.last_mut().map(|s| reverse_string(s));
    idents.reverse();
    Some((startpos, idents))
}

fn reverse_string(s: &mut String) {
    unsafe {
        let bytes = s.as_bytes_mut();
        bytes.reverse();
    }
}

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use rustyline::{
            completion::Completer, highlight::Highlighter, hint::Hinter, validate::Validator, Context,
            Helper,
        };

        impl Completer for ShellHelper<'_> {
            type Candidate = String;

            fn complete(
                &self,
                line: &str,
                pos: usize,
                _ctx: &Context,
            ) -> rustyline::Result<(usize, Vec<String>)> {
                Ok(self.complete_opt(&line[..pos])
                    .unwrap_or_else(|| (pos, vec!["\t".to_owned()])))
            }
        }

        impl Hinter for ShellHelper<'_> {
            type Hint = String;
        }
        impl Highlighter for ShellHelper<'_> {}
        impl Validator for ShellHelper<'_> {}
        impl Helper for ShellHelper<'_> {}
    }
}
