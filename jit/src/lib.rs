use std::fmt;
use std::mem;

use cranelift::prelude::*;
use cranelift_module::{Backend, Module, Linkage, FuncId};
use cranelift_simplejit::{SimpleJITBackend, SimpleJITBuilder};


struct JIT {
    builder_context: FunctionBuilderContext,
    ctx: codegen::Context,
    module: Module<SimpleJITBackend>,
}

impl JIT {
    fn new() -> Self {
        let builder = SimpleJITBuilder::new(cranelift_module::default_libcall_names());
        let module = Module::new(builder);
        Self {
            builder_context: FunctionBuilderContext::new(),
            ctx: module.make_context(),
            module,
        }
    }

    fn build_function(&mut self) -> FuncId {
        let id = self
            .module
            .declare_function("jitted", Linkage::Export, &self.ctx.func.signature)
            .unwrap();

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_context);
        let entry_block = builder.create_block();
        // builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        // builder.seal_block(entry_block);
        builder.ins().return_(&[]);
        builder.finalize();

        self.module
            .define_function(id, &mut self.ctx, &mut codegen::binemit::NullTrapSink {})
            .unwrap();

        self.module.clear_context(&mut self.ctx);

        id
    }
}

pub fn compile() -> CompiledCode {

    let mut jit = JIT::new();

    let id = jit.build_function();

    jit.module.finalize_definitions();

    let code = jit.module.get_finalized_function(id);
    CompiledCode {
        code,
        memory: jit.module.finish()
    }
}

pub struct CompiledCode {
    code: *const u8,
    memory: <SimpleJITBackend as Backend>::Product
}

impl CompiledCode {
    pub fn invoke(&self) {
        let func = unsafe { mem::transmute::<_, fn()>(self.code) };
        func()
    }
}

unsafe impl Send for CompiledCode {}

impl Drop for CompiledCode {
    fn drop(&mut self) {
        // SAFETY: The only pointer that this memory will also be dropped now
        unsafe {
            self.memory.free_memory()
        }
    }
}

impl fmt::Debug for CompiledCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[compiled code]")
    }
}
