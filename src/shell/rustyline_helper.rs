use rustpython_vm::obj::objstr::PyStringRef;
use rustpython_vm::pyobject::{PyIterable, PyResult, TryFromObject};
use rustpython_vm::scope::{NameProtocol, Scope};
use rustpython_vm::VirtualMachine;
use rustyline::{completion::Completer, highlight::Highlighter, hint::Hinter, Context, Helper};

pub struct ShellHelper<'vm> {
    vm: &'vm VirtualMachine,
    scope: Scope,
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
    pub fn new(vm: &'vm VirtualMachine, scope: Scope) -> Self {
        ShellHelper { vm, scope }
    }

    #[allow(clippy::type_complexity)]
    fn get_available_completions<'w>(
        &self,
        words: &'w [String],
    ) -> Option<(
        &'w str,
        Box<dyn Iterator<Item = PyResult<PyStringRef>> + 'vm>,
    )> {
        // the very first word and then all the ones after the dot
        let (first, rest) = words.split_first().unwrap();

        let str_iter_method = |obj, name| {
            let iter = self.vm.call_method(obj, name, vec![])?;
            PyIterable::<PyStringRef>::try_from_object(self.vm, iter)?.iter(self.vm)
        };

        if let Some((last, parents)) = rest.split_last() {
            // we need to get an attribute based off of the dir() of an object

            // last: the last word, could be empty if it ends with a dot
            // parents: the words before the dot

            let mut current = self.scope.load_global(self.vm, first)?;

            for attr in parents {
                current = self.vm.get_attribute(current.clone(), attr.as_str()).ok()?;
            }

            let current_iter = str_iter_method(&current, "__dir__").ok()?;

            Some((&last, Box::new(current_iter) as _))
        } else {
            // we need to get a variable based off of globals/builtins

            let globals = str_iter_method(self.scope.globals.as_object(), "keys").ok()?;
            let builtins = str_iter_method(&self.vm.builtins, "__dir__").ok()?;
            Some((&first, Box::new(Iterator::chain(globals, builtins)) as _))
        }
    }

    fn complete_opt(&self, line: &str) -> Option<(usize, Vec<String>)> {
        let (startpos, words) = split_idents_on_dot(line)?;

        let (word_start, iter) = self.get_available_completions(&words)?;

        let all_completions = iter
            .filter(|res| {
                res.as_ref()
                    .ok()
                    .map_or(true, |s| s.as_str().starts_with(word_start))
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
                .filter(|s| !s.as_str().starts_with('_'))
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
        completions.sort_by(|a, b| std::cmp::Ord::cmp(a.as_str(), b.as_str()));

        Some((
            startpos,
            completions
                .into_iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
        ))
    }
}

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
            .unwrap_or_else(|| (line.len(), vec!["\t".to_string()])))
    }
}

impl Hinter for ShellHelper<'_> {}
impl Highlighter for ShellHelper<'_> {}
impl Helper for ShellHelper<'_> {}
