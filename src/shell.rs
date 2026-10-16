mod helper;

use rustpython_vm::{
    PyResult, VirtualMachine,
    readline::{Readline, ReadlineResult},
    scope::Scope,
    vm::InteractiveLine,
};

/// Root-only editor/history policy, adapting into the VM's shared evaluator.
pub fn run_shell(vm: &VirtualMachine, scope: Scope) -> PyResult<()> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return vm.run_basic_interactive();
    }
    let mut repl = Readline::new(helper::ShellHelper::new(vm, scope.globals));
    let history = dirs::config_dir().map_or_else(
        || ".repl_history.txt".into(),
        |mut path| {
            path.push("rustpython");
            path.push("repl_history.txt");
            path
        },
    );
    let _ = repl.load_history(&history);
    let result = vm.run_interactive(|prompt| match repl.readline(prompt) {
        ReadlineResult::Line(line) => {
            let _ = repl.add_history_entry(line.trim_end());
            Ok(Some(InteractiveLine::Text(line)))
        }
        ReadlineResult::Eof => Ok(None),
        ReadlineResult::Interrupt => {
            Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()))
        }
        ReadlineResult::Io(_) => Ok(None),
        #[cfg(unix)]
        ReadlineResult::OsError(_) => Ok(None),
        ReadlineResult::Other(_) => Ok(None),
    });
    let _ = repl.save_history(&history);
    result
}
