use std::collections::HashMap;
use wasmer::{
    Function, FunctionEnv, FunctionEnvMut, Instance, Memory, Module, Store, Value, imports,
};

struct Ctx {
    kv: HashMap<Vec<u8>, Vec<u8>>,
    mem: Option<Memory>,
}

/// kp and kl are the key pointer and length in wasm memory, vp and vl are for the return value
/// if read value is bigger than vl then it will be truncated to vl, returns read bytes
fn kv_get(mut ctx: FunctionEnvMut<Ctx>, kp: i32, kl: i32, vp: i32, vl: i32) -> i32 {
    let (c, s) = ctx.data_and_store_mut();
    let mut key = vec![0u8; kl as usize];
    if c.mem
        .as_ref()
        .unwrap()
        .view(&s)
        .read(kp as u64, &mut key)
        .is_err()
    {
        return -1;
    }
    match c.kv.get(&key) {
        Some(val) => {
            let len = val.len().min(vl as usize);
            if c.mem
                .as_ref()
                .unwrap()
                .view(&s)
                .write(vp as u64, &val[..len])
                .is_err()
            {
                return -1;
            }
            len as i32
        }
        None => 0,
    }
}

/// kp and kl are the key pointer and length in wasm memory, vp and vl are for the value
fn kv_put(mut ctx: FunctionEnvMut<Ctx>, kp: i32, kl: i32, vp: i32, vl: i32) -> i32 {
    let (c, s) = ctx.data_and_store_mut();
    let mut key = vec![0u8; kl as usize];
    let mut val = vec![0u8; vl as usize];
    let m = c.mem.as_ref().unwrap().view(&s);
    if m.read(kp as u64, &mut key).is_err() || m.read(vp as u64, &mut val).is_err() {
        return -1;
    }
    c.kv.insert(key, val);
    0
}

// // p and l are the buffer pointer and length in wasm memory.
// fn get_code(mut ctx:FunctionEnvMut<Ctx>, p: i32, l: i32) -> i32 {
//     let file_name = std::env::args().nth(2).expect("file_name is not given");
//     let code : String = std::fs::read_to_string(file_name).expect("file read failed");
//     if code.len() > l as usize {
//         eprintln!("code is too long");
//         return -1;
//     }

//     let (c, s) = ctx.data_and_store_mut();
//     let m = c.mem.as_ref().unwrap().view(&s);
//     if m.write(p as u64, code.as_bytes()).is_err() {
//         return -2;
//     }

//     0
// }

// p and l are the message pointer and length in wasm memory.
fn print(mut ctx: FunctionEnvMut<Ctx>, p: i32, l: i32) -> i32 {
    let (c, s) = ctx.data_and_store_mut();
    let mut msg = vec![0u8; l as usize];
    let m = c.mem.as_ref().unwrap().view(&s);
    if m.read(p as u64, &mut msg).is_err() {
        return -1;
    }
    let s = std::str::from_utf8(&msg).expect("print got non-utf8 str");
    println!("{s}");
    0
}

fn main() {
    let mut store = Store::default();
    let module = Module::new(
        &store,
        &std::fs::read(&std::env::args().nth(1).unwrap()).unwrap(),
    )
    .unwrap();

    // Prepare initial KV store with Python code
    let mut initial_kv = HashMap::new();
    initial_kv.insert(
        b"code".to_vec(),
        b"a=10;b='str';f'{a}{b}'".to_vec(), // Python code to execute
    );

    let env = FunctionEnv::new(
        &mut store,
        Ctx {
            kv: initial_kv,
            mem: None,
        },
    );
    let imports = imports! {
        "env" => {
            "kv_get" => Function::new_typed_with_env(&mut store, &env, kv_get),
            "kv_put" => Function::new_typed_with_env(&mut store, &env, kv_put),
            // "get_code" => Function::new_typed_with_env(&mut store, &env, get_code),
            "print" => Function::new_typed_with_env(&mut store, &env, print),
        }
    };
    let inst = Instance::new(&mut store, &module, &imports).unwrap();
    env.as_mut(&mut store).mem = inst.exports.get_memory("memory").ok().cloned();
    let res = inst
        .exports
        .get_function("eval")
        .unwrap()
        // TODO: actually pass source code
        .call(&mut store, &[wasmer::Value::I32(0), wasmer::Value::I32(0)])
        .unwrap();
    println!(
        "Result: {}",
        match res[0] {
            Value::I32(v) => v,
            _ => -1,
        }
    );
}
