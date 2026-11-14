use std::{collections::HashMap, path::PathBuf, process::ExitCode};

use wasmer::{
    AsStoreRef, Function, FunctionEnv, FunctionEnvMut, Instance, Memory, Module, RuntimeError,
    Store, imports,
};

const USAGE: &str = "usage: wasm-runtime MODULE.wasm (-c CODE | FILE.py)";

struct Context {
    memory: Option<Memory>,
    kv: HashMap<Vec<u8>, Vec<u8>>,
}

impl Context {
    fn memory(&self) -> Result<&Memory, RuntimeError> {
        self.memory
            .as_ref()
            .ok_or_else(|| RuntimeError::new("guest memory is not initialized"))
    }
}

fn read_memory(
    context: &Context,
    store: &impl AsStoreRef,
    ptr: u32,
    len: u32,
) -> Result<Vec<u8>, RuntimeError> {
    let mut bytes = vec![0; len as usize];
    context
        .memory()?
        .view(store)
        .read(u64::from(ptr), &mut bytes)
        .map_err(|err| RuntimeError::new(err.to_string()))?;
    Ok(bytes)
}

fn print(mut ctx: FunctionEnvMut<Context>, ptr: u32, len: u32) -> Result<i32, RuntimeError> {
    let (context, store) = ctx.data_and_store_mut();
    let bytes = read_memory(context, &store, ptr, len)?;
    let message = std::str::from_utf8(&bytes).map_err(|err| RuntimeError::new(err.to_string()))?;
    println!("{message}");
    Ok(0)
}

fn kv_put(
    mut ctx: FunctionEnvMut<Context>,
    key_ptr: u32,
    key_len: u32,
    value_ptr: u32,
    value_len: u32,
) -> Result<i32, RuntimeError> {
    let (context, store) = ctx.data_and_store_mut();
    let key = read_memory(context, &store, key_ptr, key_len)?;
    let value = read_memory(context, &store, value_ptr, value_len)?;
    context.kv.insert(key, value);
    Ok(0)
}

fn kv_get(
    mut ctx: FunctionEnvMut<Context>,
    key_ptr: u32,
    key_len: u32,
    buf_ptr: u32,
    buf_len: u32,
) -> Result<i32, RuntimeError> {
    let (context, store) = ctx.data_and_store_mut();
    let key = read_memory(context, &store, key_ptr, key_len)?;
    let Some(value) = context.kv.get(&key) else {
        return Ok(-1);
    };
    let len = i32::try_from(value.len()).map_err(|err| RuntimeError::new(err.to_string()))?;
    if value.len() <= buf_len as usize {
        context
            .memory()?
            .view(&store)
            .write(u64::from(buf_ptr), value)
            .map_err(|err| RuntimeError::new(err.to_string()))?;
    }
    Ok(len)
}

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let module_path = PathBuf::from(args.next().ok_or(USAGE)?);
    let source_arg = args.next().ok_or(USAGE)?;
    let source = if source_arg == "-c" {
        args.next()
            .ok_or(USAGE)?
            .into_string()
            .map_err(|_| "-c source is not valid UTF-8")?
    } else if source_arg.as_encoded_bytes().starts_with(b"-") {
        return Err(USAGE.into());
    } else {
        std::fs::read_to_string(&source_arg)?
    };
    if args.next().is_some() {
        return Err(USAGE.into());
    }

    let mut store = Store::default();
    let module = Module::new(&store, std::fs::read(&module_path)?)?;
    // Fail closed if a future build accidentally acquires OS or JS imports.
    for import in module.imports() {
        if import.module() != "env" || !matches!(import.name(), "print" | "kv_get" | "kv_put") {
            return Err(format!("unexpected import: {}.{}", import.module(), import.name()).into());
        }
    }
    let env = FunctionEnv::new(
        &mut store,
        Context {
            memory: None,
            kv: HashMap::new(),
        },
    );
    let imports = imports! {
        "env" => {
            "print" => Function::new_typed_with_env(&mut store, &env, print),
            "kv_get" => Function::new_typed_with_env(&mut store, &env, kv_get),
            "kv_put" => Function::new_typed_with_env(&mut store, &env, kv_put),
        }
    };
    let instance = Instance::new(&mut store, &module, &imports)?;
    let memory = instance.exports.get_memory("memory")?.clone();
    env.as_mut(&mut store).memory = Some(memory.clone());
    let alloc = instance
        .exports
        .get_typed_function::<u32, u32>(&store, "alloc_source")?;
    let eval = instance
        .exports
        .get_typed_function::<(u32, u32), i32>(&store, "eval")?;
    let len = u32::try_from(source.len())?;
    let ptr = alloc.call(&mut store, len)?;
    memory
        .view(&store)
        .write(u64::from(ptr), source.as_bytes())?;
    Ok(eval.call(&mut store, ptr, len)?)
}

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(1) => ExitCode::from(1),
        Ok(status) => {
            eprintln!("guest failed with status {status}");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
