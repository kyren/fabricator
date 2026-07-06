use std::{fs::File, io::Read, path::PathBuf, process::ExitCode};

use anyhow::Error;
use clap::{Parser, Subcommand};
use fabricator_compiler::{
    CompileError, CompileSettings,
    compiler::{CompileErrorKind, Compiler, ImportItems},
    parser::{ParseError, ParseErrorKind},
};
use fabricator_stdlib::{StdlibContext as _, string::debug_value};
use fabricator_vm as vm;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    #[arg(long, short = 'O', default_value_t = 2)]
    opt_level: u8,
}

#[derive(Subcommand)]
enum Command {
    Run { path: PathBuf },
    Dump { path: PathBuf },
    Repl,
}

fn main() -> Result<ExitCode, Error> {
    let cli = Cli::parse();
    let mut interpreter = vm::Interpreter::new();
    match cli.command {
        Command::Run { path } => {
            let mut code = String::new();
            File::open(&path)?.read_to_string(&mut code)?;

            let settings = CompileSettings::from_path(&path).set_optimization_passes(cli.opt_level);

            interpreter.enter(|ctx| {
                let output = Compiler::compile_chunk(
                    ctx,
                    "",
                    ImportItems::with_magic(&ctx, ctx.stdlib()),
                    settings,
                    path.to_string_lossy().into_owned(),
                    &code,
                )?;
                let closure =
                    vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();

                let thread = vm::Thread::new(&ctx);
                Ok(match thread.run(ctx, closure) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(err) => {
                        println!("error: {}", err);
                        ExitCode::FAILURE
                    }
                })
            })
        }
        Command::Dump { path } => {
            let mut code = String::new();
            File::open(&path)?.read_to_string(&mut code)?;

            let settings = CompileSettings::from_path(&path).set_optimization_passes(cli.opt_level);

            interpreter.enter(|ctx| {
                let output = Compiler::compile_chunk(
                    ctx,
                    "",
                    ImportItems::with_magic(&ctx, ctx.stdlib()),
                    settings,
                    path.to_string_lossy().into_owned(),
                    &code,
                )?;

                for (ir, proto) in output.all_prototypes {
                    let chunk = proto.chunk();
                    match proto.reference() {
                        vm::FunctionRef::Named(ref_name, span) => {
                            println!(
                                "==[Function named {ref_name} at line {}]==",
                                chunk.line_number(span.start())
                            );
                        }
                        vm::FunctionRef::Expression(span) => {
                            println!(
                                "==[Function expression at line {}]==",
                                chunk.line_number(span.start())
                            );
                        }
                        vm::FunctionRef::Chunk => {
                            println!("==[Chunk function]==");
                        }
                    }
                    println!();
                    println!("IR: {:#?}", ir);
                    println!("Bytecode: {:#?}", proto);
                }
                Ok(ExitCode::SUCCESS)
            })
        }
        Command::Repl => {
            let mut editor = rustyline::DefaultEditor::new()?;
            let thread = interpreter.enter(|ctx| ctx.stash(vm::Thread::new(&ctx)));

            let settings = CompileSettings::strict().set_optimization_passes(cli.opt_level);

            let mut imports =
                interpreter.enter(|ctx| ctx.stash(ImportItems::with_magic(&ctx, ctx.stdlib())));

            fn is_end_of_stream_err(e: &CompileError) -> bool {
                matches!(
                    e,
                    CompileError {
                        kind: CompileErrorKind::Parsing(ParseError {
                            kind: ParseErrorKind::EndOfStream { .. },
                            ..
                        }),
                        ..
                    }
                )
            }

            loop {
                let mut prompt = "> ";
                let mut line = String::new();

                interpreter.enter(|ctx| -> Result<(), Error> {
                    loop {
                        let read = editor.readline(prompt)?;
                        let read_empty = read.trim().is_empty();
                        if !read_empty {
                            if !line.is_empty() {
                                // Separate input lines in the input to the parser
                                line.push('\n');
                            }
                            line.push_str(&read);
                        }

                        let try_compile = |code: &str| {
                            Compiler::compile_chunk(
                                ctx,
                                "",
                                ctx.fetch(&imports),
                                settings,
                                "line-input",
                                code,
                            )
                        };

                        let compile_res = try_compile(&line)
                            .or_else(|_| try_compile(&format!("return {line};")))
                            .or_else(|_| try_compile(&format!("{line};")));

                        match compile_res {
                            Ok(output) => {
                                imports = ctx.stash(output.exported_imports);

                                let closure = vm::Closure::new(
                                    &ctx,
                                    output.chunk_prototype,
                                    vm::Value::Undefined,
                                )
                                .unwrap();

                                let thread = ctx.fetch(&thread);
                                thread.exec(ctx, |mut exec| {
                                    if let Err(err) = exec.call_closure(ctx, closure) {
                                        eprintln!("{}", err);
                                    } else {
                                        let stack = exec.stack();
                                        if !stack.is_empty() {
                                            let mut ret_iter = stack.iter().peekable();
                                            while let Some(r) = ret_iter.next() {
                                                print!("{:?}", debug_value(ctx, r));
                                                if ret_iter.peek().is_some() {
                                                    print!("\t");
                                                }
                                            }
                                            println!();
                                        }
                                    }
                                });

                                editor.add_history_entry(line)?;
                                break;
                            }
                            Err(err) if is_end_of_stream_err(&err) => {
                                prompt = ">> ";
                            }
                            Err(err) => {
                                editor.add_history_entry(line)?;
                                eprintln!("{}", err);
                                break;
                            }
                        }
                    }
                    Ok(())
                })?;
                interpreter.gc_collect_debt();
            }
        }
    }
}
