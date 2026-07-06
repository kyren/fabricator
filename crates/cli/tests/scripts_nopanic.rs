use std::{
    fs::{File, read_dir},
    io::{self, Write, stdout},
};

use anyhow::Error;
use fabricator_cli::TestingStdlibContext as _;
use fabricator_compiler as compiler;
use fabricator_vm as vm;
use gc_arena::Collect;
use thiserror::Error;

fn run_code(
    name: &str,
    code: &str,
    compile_settings: compiler::CompileSettings,
) -> Result<bool, Error> {
    const FRAME_LIMIT: u32 = 64;
    const INST_LIMIT: u32 = 32678;

    #[derive(Debug, Error)]
    #[error("vm limit reached")]
    struct VmLimitError;

    #[derive(Default, Collect)]
    #[collect(require_static)]
    struct VmLimiter {
        frames_available: u32,
        insts_available: u32,
    }

    impl VmLimiter {
        fn new() -> Self {
            Self {
                frames_available: FRAME_LIMIT,
                insts_available: INST_LIMIT,
            }
        }
    }

    impl<'gc> vm::Hook<'gc> for VmLimiter {
        fn on_call(
            &mut self,
            _ctx: vm::Context<'gc>,
            _backtrace: vm::Backtrace<'gc, '_>,
        ) -> Result<(), vm::RuntimeError> {
            if let Some(dec) = self.frames_available.checked_sub(1) {
                self.frames_available = dec;
                Ok(())
            } else {
                Err(VmLimitError.into())
            }
        }

        fn on_return(&mut self, _ctx: vm::Context<'gc>, _backtrace: vm::Backtrace<'gc, '_>) {
            self.frames_available += 1;
        }

        fn on_step(
            &mut self,
            _ctx: vm::Context<'gc>,
            instruction_count: u32,
        ) -> Result<u32, vm::RuntimeError> {
            self.insts_available = self.insts_available.saturating_sub(instruction_count);
            if self.insts_available == 0 {
                Err(VmLimitError.into())
            } else {
                Ok(self.insts_available)
            }
        }
    }

    let interpreter = vm::Interpreter::new();

    interpreter.enter(|ctx| {
        let output = compiler::Compiler::compile_chunk(
            ctx,
            "default",
            compiler::ImportItems::with_magic(&ctx, ctx.testing_stdlib()),
            compile_settings,
            name,
            code,
        )?;
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();

        let thread = vm::Thread::new(&ctx);
        thread.set_hook(&ctx, VmLimiter::new());
        thread.exec(ctx, |mut exec| {
            exec.call(ctx, closure)?;
            Ok(exec.stack().get(0) == vm::Value::Boolean(true))
        })
    })
}

fn try_scripts(dir: &str) {
    let _ = writeln!(stdout(), "trying all scripts in {dir:?}");

    for dir in read_dir(dir).expect("could not list dir contents") {
        let path = dir.expect("could not read dir entry").path();
        let code = io::read_to_string(File::open(&path).unwrap()).unwrap();
        if let Some(ext) = path.extension() {
            if ext.eq_ignore_ascii_case("fml") || ext.eq_ignore_ascii_case("gml") {
                let _ = writeln!(stdout(), "trying {:?}", path);
                let _ = run_code(
                    path.to_string_lossy().as_ref(),
                    &code,
                    if ext.eq_ignore_ascii_case("gml") {
                        compiler::CompileSettings::compat()
                    } else {
                        compiler::CompileSettings::strict()
                    },
                );
            }
        } else {
            let _ = writeln!(stdout(), "skipping file {:?}", path);
        }
    }
}

#[test]
fn test_scripts_nopanic() {
    try_scripts("./tests/scripts_nopanic");
}
