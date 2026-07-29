use std::{
    fs::{File, read_dir},
    io::{self, Write, stdout},
};

use anyhow::Error;
use fabricator_cli::TestingStdlibContext as _;
use fabricator_compiler as compiler;
use fabricator_vm as vm;

pub fn run_code(
    name: &str,
    code: &str,
    compile_settings: compiler::CompileSettings,
) -> Result<bool, Error> {
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
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, None).unwrap();

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
        if let Some(ext) = path.extension() {
            if ext.eq_ignore_ascii_case("fml") || ext.eq_ignore_ascii_case("gml") {
                let code = io::read_to_string(File::open(&path).unwrap()).unwrap();
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
    if !run_tests("./tests/scripts_success") {
        panic!("one or more errors occurred");
    }
}
