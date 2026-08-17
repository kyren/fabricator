#![no_main]

use std::str;

use anyhow::Error;
use fabricator_cli::TestingStdlibContext as _;
use fabricator_compiler::{
    CompileSettings,
    compiler::{Compiler, ImportItems},
};
use fabricator_vm as vm;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|code: &str| {
    let interpreter = vm::Interpreter::new();
    let settings = CompileSettings::modern();

    let _ = interpreter.enter(|ctx| -> Result<(), Error> {
        let output = Compiler::compile_chunk(
            ctx,
            "",
            ImportItems::with_magic(&ctx, ctx.testing_stdlib()),
            settings,
            "<fuzzer input>",
            code,
        )?;
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();

        let thread = vm::Thread::new(&ctx);
        thread.run(ctx, closure)?;

        Ok(())
    });
});
