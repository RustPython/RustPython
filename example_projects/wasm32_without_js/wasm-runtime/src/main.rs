use std::{
    borrow::Cow,
    io::{self, Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::mpsc,
    time::Duration,
};
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Instance, Memory, Module, Store, imports};

const MAX_SOURCE_SIZE: usize = 16 * 1024 * 1024;

#[derive(Default)]
struct Context {
    memory: Option<Memory>,
    output: Option<String>,
}

struct Report {
    status: &'static str,
    output: String,
    exit_code: u8,
}

impl Report {
    fn new(status: &'static str, output: impl Into<String>, exit_code: u8) -> Self {
        Self {
            status,
            output: output.into(),
            exit_code,
        }
    }

    fn emit(&self) -> io::Result<()> {
        let record = serde_json::json!({"status": self.status, "output": self.output});
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{record}")?;
        stdout.flush()
    }
}

enum Source {
    Command(Vec<u8>),
    File(PathBuf),
}

impl Source {
    fn read(&self) -> Result<Cow<'_, [u8]>, String> {
        let source = match self {
            Self::Command(source) => Cow::Borrowed(source.as_slice()),
            Self::File(path) => {
                let mut source = Vec::new();
                std::fs::File::open(path)
                    .and_then(|file| {
                        file.take(MAX_SOURCE_SIZE as u64 + 1)
                            .read_to_end(&mut source)
                    })
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                Cow::Owned(source)
            }
        };
        if source.len() > MAX_SOURCE_SIZE {
            return Err(format!("source exceeds {MAX_SOURCE_SIZE} bytes"));
        }
        Ok(source)
    }
}

struct Options {
    module: PathBuf,
    source: Source,
    timeout: Duration,
}

fn options() -> Result<Options, String> {
    let mut args = std::env::args_os().skip(1);
    let module = args
        .next()
        .ok_or("usage: wasm-runtime MODULE.wasm [-c CODE | FILE.py] [--timeout SECONDS]")?;
    let mut source = None;
    let mut timeout = Duration::from_secs(60);
    while let Some(arg) = args.next() {
        if arg == "--timeout" {
            let value = args.next().ok_or("--timeout requires seconds")?;
            timeout = value
                .to_str()
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
                .and_then(|value| Duration::try_from_secs_f64(value).ok())
                .filter(|value| !value.is_zero())
                .ok_or("--timeout must be a finite positive number of seconds")?;
        } else {
            if source.is_some() {
                return Err("provide only one source file or -c command".into());
            }
            source = Some(if arg == "-c" {
                Source::Command(
                    args.next()
                        .ok_or("-c requires Python source")?
                        .into_string()
                        .map_err(|_| "-c source is not valid UTF-8")?
                        .into_bytes(),
                )
            } else if arg.to_string_lossy().starts_with('-') {
                return Err(format!("unknown option: {}", arg.to_string_lossy()));
            } else {
                Source::File(arg.into())
            });
        }
    }
    let source = source.unwrap_or_else(|| Source::Command(b"1 + 3".to_vec()));
    Ok(Options {
        module: module.into(),
        source,
        timeout,
    })
}

// Only an explicit message callback is provided. The guest has no JS, WASI,
// filesystem, clock, entropy, or other operating-system imports.
fn print(
    mut ctx: FunctionEnvMut<Context>,
    ptr: u32,
    len: u32,
) -> Result<i32, wasmer::RuntimeError> {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context
        .memory
        .as_ref()
        .ok_or_else(|| wasmer::RuntimeError::new("guest memory is not initialized"))?
        .view(&store);
    let end = u64::from(ptr) + u64::from(len);
    if end > memory.data_size() {
        return Err(wasmer::RuntimeError::new("output is outside guest memory"));
    }
    let mut bytes = vec![0; len as usize];
    memory
        .read(u64::from(ptr), &mut bytes)
        .map_err(|err| wasmer::RuntimeError::new(err.to_string()))?;
    let message =
        std::str::from_utf8(&bytes).map_err(|err| wasmer::RuntimeError::new(err.to_string()))?;
    if context.output.is_some() {
        return Err(wasmer::RuntimeError::new(
            "guest reported more than one result",
        ));
    }
    context.output = Some(message.to_owned());
    Ok(0)
}

fn run(options: &Options) -> Result<Report, Box<dyn std::error::Error>> {
    let source = match options.source.read() {
        Ok(source) => source,
        Err(error) => return Ok(Report::new("input_error", error, 2)),
    };
    let mut store = Store::default();
    let module = Module::new(&store, std::fs::read(&options.module)?)?;
    // Fail closed if a future build accidentally acquires OS or JS imports.
    for import in module.imports() {
        if import.module() != "env" || import.name() != "print" {
            return Err(format!("unexpected import: {}.{}", import.module(), import.name()).into());
        }
    }
    let env = FunctionEnv::new(&mut store, Context::default());
    let imports = imports! {
        "env" => {
            "print" => Function::new_typed_with_env(&mut store, &env, print),
        }
    };
    let instance = Instance::new(&mut store, &module, &imports)?;
    let memory = instance.exports.get_memory("memory")?.clone();
    env.as_mut(&mut store).memory = Some(memory.clone());
    let alloc = instance
        .exports
        .get_typed_function::<u32, u32>(&store, "alloc_source")?;
    let free = instance
        .exports
        .get_typed_function::<(u32, u32), ()>(&store, "free_source")?;
    let eval = instance
        .exports
        .get_typed_function::<(u32, u32), i32>(&store, "eval")?;
    let len = u32::try_from(source.len())?;
    let ptr = alloc.call(&mut store, len)?;
    memory.view(&store).write(u64::from(ptr), &source)?;
    let status = eval.call(&mut store, ptr, len)?;
    free.call(&mut store, ptr, len)?;
    let (status, exit_code) = match status {
        0 => ("ok", 0),
        1 => ("python_error", 1),
        2 => ("input_error", 2),
        code => return Err(format!("unknown guest status: {code}").into()),
    };
    let output = env
        .as_mut(&mut store)
        .output
        .take()
        .ok_or("guest did not report a result")?;
    Ok(Report::new(status, output, exit_code))
}

fn finish(report: Report) -> ExitCode {
    if let Err(error) = report.emit() {
        eprintln!("could not write result: {error}");
        return ExitCode::from(2);
    }
    ExitCode::from(report.exit_code)
}

fn main() -> ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(error) => {
            return finish(Report::new("input_error", error, 2));
        }
    };
    // The deadline includes input reads, compilation, and execution. Terminating
    // the host stops uncooperative guests without exposing guest OS APIs.
    let (done, wait) = mpsc::channel();
    let timeout = options.timeout;
    let watchdog = std::thread::spawn(move || {
        if matches!(
            wait.recv_timeout(timeout),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            let _ = Report::new("timeout", "WASM execution exceeded its deadline", 124).emit();
            std::process::exit(124);
        }
    });
    let report =
        run(&options).unwrap_or_else(|error| Report::new("runtime_error", error.to_string(), 2));
    let _ = done.send(());
    watchdog.join().unwrap();
    finish(report)
}
