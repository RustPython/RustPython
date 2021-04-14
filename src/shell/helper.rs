use rustpython_vm::builtins::{PyDictRef, PyStrRef};
use rustpython_vm::pyobject::{BorrowValue, PyIterable, PyResult, TryFromObject};
use rustpython_vm::VirtualMachine;

pub struct ShellHelper<'vm> {
    vm: &'vm VirtualMachine,
    globals: PyDictRef,
}

fn reverse_string(s: &mut String) {
    let rev = s.chars().rev().collect();
    *s = rev;
}

fn split_idents_on_dot(line: &str) -> Option<(usize, Vec<String>)> {
    let mut words = vec![String::new()];
    let mut startpos = 0;
    for (i, c) in line.chars().rev().enumerate() {
        match c {
            '.' => {
                // check for a double dot
                if i != 0 && words.last().map_or(false, |s| s.is_empty()) {
                    return None;
                }
                reverse_string(words.last_mut().unwrap());
                if words.len() == 1 {
                    startpos = line.len() - i;
                }
                words.push(String::new());
            }
            c if c.is_alphanumeric() || c == '_' => words.last_mut().unwrap().push(c),
            _ => {
                if words.len() == 1 {
                    if words.last().unwrap().is_empty() {
                        return None;
                    }
                    startpos = line.len() - i;
                }
                break;
            }
        }
    }
    if words == [String::new()] {
        return None;
    }
    reverse_string(words.last_mut().unwrap());
    words.reverse();

    Some((startpos, words))
}

impl<'vm> ShellHelper<'vm> {
    pub fn new(vm: &'vm VirtualMachine, globals: PyDictRef) -> Self {
        ShellHelper { vm, globals }
    }

    #[allow(clippy::type_complexity)]
    fn get_available_completions<'w>(
        &self,
        words: &'w [String],
    ) -> Option<(&'w str, impl Iterator<Item = PyResult<PyStrRef>> + 'vm)> {
        // the very first word and then all the ones after the dot
        let (first, rest) = words.split_first().unwrap();

        let str_iter_method = |obj, name| {
            let iter = self.vm.call_special_method(obj, name, ())?;
            PyIterable::<PyStrRef>::try_from_object(self.vm, iter)?.iter(self.vm)
        };

        let (word_start, iter1, iter2) = if let Some((last, parents)) = rest.split_last() {
            // we need to get an attribute based off of the dir() of an object

            // last: the last word, could be empty if it ends with a dot
            // parents: the words before the dot

            let mut current = self
                .globals
                .get_item_option(first.as_str(), self.vm)
                .ok()??;

            for attr in parents {
                current = self.vm.get_attribute(current.clone(), attr.as_str()).ok()?;
            }

            let current_iter = str_iter_method(current, "__dir__").ok()?;

            (last, current_iter, None)
        } else {
            // we need to get a variable based off of globals/builtins

            let globals = str_iter_method(self.globals.as_object().clone(), "keys").ok()?;
            let builtins = str_iter_method(self.vm.builtins.clone(), "__dir__").ok()?;
            (first, globals, Some(builtins))
        };
        Some((word_start, iter1.chain(iter2.into_iter().flatten())))
    }

    fn complete_opt(&self, line: &str) -> Option<(usize, Vec<String>)> {
        let (startpos, words) = split_idents_on_dot(line)?;

        let (word_start, iter) = self.get_available_completions(&words)?;

        let all_completions = iter
            .filter(|res| {
                res.as_ref()
                    .ok()
                    .map_or(true, |s| s.borrow_value().starts_with(word_start))
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let mut completions = if word_start.starts_with('_') {
            // if they're already looking for something starting with a '_', just give
            // them all the completions
            all_completions
        } else {
            // only the completions that don't start with a '_'
            let no_underscore = all_completions
                .iter()
                .cloned()
                .filter(|s| !s.borrow_value().starts_with('_'))
                .collect::<Vec<_>>();

            // if there are only completions that start with a '_', give them all of the
            // completions, otherwise only the ones that don't start with '_'
            if no_underscore.is_empty() {
                all_completions
            } else {
                no_underscore
            }
        };

        // sort the completions alphabetically
        completions.sort_by(|a, b| std::cmp::Ord::cmp(a.borrow_value(), b.borrow_value()));

        Some((
            startpos,
            completions
                .into_iter()
                .map(|s| s.borrow_value().to_owned())
                .collect(),
        ))
    }
}

cfg_if::cfg_if! {
    if #[cfg(not(target_os = "wasi"))] {
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
                Ok(self
                    .complete_opt(&line[0..pos])
                    // as far as I can tell, there's no better way to do both completion
                    // and indentation (or even just indentation)
                    .unwrap_or_else(|| (line.len(), vec!["\t".to_owned()])))
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
