use rustpython_vm::obj::objstr::PyStringRef;
use rustpython_vm::pyobject::{PyIterable, PyResult, TryFromObject};
use rustpython_vm::scope::{NameProtocol, Scope};
use rustpython_vm::VirtualMachine;
use rustyline::{completion::Completer, highlight::Highlighter, hint::Hinter, Context, Helper};

pub struct ShellHelper<'a> {
    vm: &'a VirtualMachine,
    scope: Scope,
}

fn reverse_string(s: &mut String) {
    let rev = s.chars().rev().collect();
    *s = rev;
}

fn extract_words(line: &str) -> Option<(usize, Vec<String>)> {
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
    reverse_string(words.last_mut().unwrap());
    words.reverse();
    Some((startpos, words))
}

impl<'a> ShellHelper<'a> {
    pub fn new(vm: &'a VirtualMachine, scope: Scope) -> Self {
        ShellHelper { vm, scope }
    }

    // fn get_words

    fn complete_opt(&self, line: &str) -> Option<(usize, Vec<String>)> {
        let (startpos, words) = extract_words(line)?;

        // the very first word and then all the ones after the dot
        let (first, rest) = words.split_first().unwrap();

        let str_iter = |obj| {
            PyIterable::<PyStringRef>::try_from_object(self.vm, obj)
                .ok()?
                .iter(self.vm)
                .ok()
        };

        type StrIter<'a> = Box<dyn Iterator<Item = PyResult<PyStringRef>> + 'a>;

        let (iter, prefix) = if let Some((last, parents)) = rest.split_last() {
            // we need to get an attribute based off of the dir() of an object

            // last: the last word, could be empty if it ends with a dot
            // parents: the words before the dot

            let mut current = self.scope.load_global(self.vm, first)?;

            for attr in parents {
                current = self.vm.get_attribute(current.clone(), attr.as_str()).ok()?;
            }

            (
                Box::new(str_iter(
                    self.vm.call_method(&current, "__dir__", vec![]).ok()?,
                )?) as StrIter,
                last.as_str(),
            )
        } else {
            // we need to get a variable based off of globals/builtins

            let globals = str_iter(
                self.vm
                    .call_method(self.scope.globals.as_object(), "keys", vec![])
                    .ok()?,
            )?;
            let iter = if first.as_str().is_empty() {
                // only show globals that don't start with a  '_'
                Box::new(globals.filter(|r| {
                    r.as_ref()
                        .ok()
                        .map_or(true, |s| !s.as_str().starts_with('_'))
                })) as StrIter
            } else {
                // show globals and builtins
                Box::new(
                    globals.chain(str_iter(
                        self.vm
                            .call_method(&self.vm.builtins, "__dir__", vec![])
                            .ok()?,
                    )?),
                ) as StrIter
            };
            (iter, first.as_str())
        };
        let completions = iter
            .filter(|res| {
                res.as_ref()
                    .ok()
                    .map_or(true, |s| s.as_str().starts_with(prefix))
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let no_underscore = completions
            .iter()
            .cloned()
            .filter(|s| !prefix.starts_with('_') && !s.as_str().starts_with('_'))
            .collect::<Vec<_>>();
        let mut completions = if no_underscore.is_empty() {
            completions
        } else {
            no_underscore
        };
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
        if pos != line.len() {
            return Ok((0, vec![]));
        }
        Ok(self.complete_opt(line).unwrap_or((0, vec![])))
    }
}

impl Hinter for ShellHelper<'_> {}
impl Highlighter for ShellHelper<'_> {}
impl Helper for ShellHelper<'_> {}
