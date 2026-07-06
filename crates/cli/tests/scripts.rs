use std::{
    error::Error,
    fs::{File, read_dir},
    io::{self, Write, stdout},
};

use fabricator_compiler as compiler;
use fabricator_stdlib::StdlibContext as _;
use fabricator_vm as vm;
use gc_arena::Gc;

pub fn testing_stdlib<'gc>(ctx: vm::Context<'gc>) -> Gc<'gc, vm::MagicSet<'gc>> {
    let mut lib = vm::MagicSet::new();
    lib.merge(&ctx.stdlib());

    let assert = vm::Callback::from_fn(&ctx, |_, mut exec| {
        let stack = exec.stack();
        for i in 0..stack.len() {
            if !stack.get(i).cast_bool() {
                return Err(vm::RuntimeError::msg("assert failed"));
            }
        }
        Ok(())
    });
    lib.insert(
        ctx.intern("assert"),
        vm::magic::MagicConstant::new_ptr(&ctx, assert),
    );

    let black_box = vm::Callback::from_fn(&ctx, |_, _| Ok(()));
    lib.insert(
        ctx.intern("black_box"),
        vm::magic::MagicConstant::new_ptr(&ctx, black_box),
    );

    Gc::new(&ctx, lib)
}

fn run_code(
    name: &str,
    code: &str,
    compile_settings: compiler::CompileSettings,
) -> Result<bool, Box<dyn Error>> {
    let interpreter = vm::Interpreter::new();

    interpreter.enter(|ctx| {
        let output = compiler::Compiler::compile_chunk(
            ctx,
            "default",
            compiler::ImportItems::with_magic(&ctx, testing_stdlib(ctx)),
            compile_settings,
            name,
            code,
        )?;
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();

        let thread = vm::Thread::new(&ctx);
        thread.exec(ctx, |mut exec| {
            exec.call(ctx, closure)?;
            Ok(exec.stack().get(0) == vm::Value::Boolean(true))
        })
    })
}

fn run_tests(dir: &str) -> bool {
    let _ = writeln!(stdout(), "running all test scripts in {dir:?}");

    let mut all_passed = true;
    for dir in read_dir(dir).expect("could not list dir contents") {
        let path = dir.expect("could not read dir entry").path();
        let code = io::read_to_string(File::open(&path).unwrap()).unwrap();
        if let Some(ext) = path.extension() {
            if ext.eq_ignore_ascii_case("fml") || ext.eq_ignore_ascii_case("gml") {
                let _ = writeln!(stdout(), "running {:?}", path);
                match run_code(
                    path.to_string_lossy().as_ref(),
                    &code,
                    if ext.eq_ignore_ascii_case("gml") {
                        compiler::CompileSettings::compat()
                    } else {
                        compiler::CompileSettings::strict()
                    },
                ) {
                    Ok(ret_true) => {
                        if !ret_true {
                            let _ = writeln!(stdout(), "script {:?} did not return `true`", path);
                            all_passed = false;
                        }
                    }
                    Err(err) => {
                        let _ = writeln!(stdout(), "error encountered running {:?}: {}", path, err);
                        all_passed = false;
                    }
                }
            }
        } else {
            let _ = writeln!(stdout(), "skipping file {:?}", path);
        }
    }
    all_passed
}

#[test]
fn test_scripts() {
    if !run_tests("./tests/scripts") {
        panic!("one or more errors occurred");
    }
}
