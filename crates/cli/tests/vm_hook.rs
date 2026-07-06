use std::{cell::Cell, error::Error, fmt, rc::Rc};

use fabricator_compiler as compiler;
use fabricator_stdlib::{StdlibContext as _, util::MagicExt as _};
use fabricator_vm as vm;
use gc_arena::{Collect, Gc};

#[test]
fn test_vm_call_return_hooks() {
    let interpreter = vm::Interpreter::new();

    interpreter.enter(|ctx| {
        let mut magic = vm::MagicSet::new();
        magic.merge(&ctx.stdlib());

        magic.insert_constant(
            ctx,
            "test_callback",
            vm::Callback::from_fn(&ctx, |_ctx, mut exec| {
                exec.stack().clear();
                Ok(())
            }),
        );

        let magic = Gc::new(&ctx, magic);

        let output = compiler::Compiler::compile_chunk(
            ctx,
            "default",
            compiler::ImportItems::with_magic(&ctx, magic),
            compiler::CompileSettings::strict(),
            "vm hook test",
            r#"
                function test1() {
                    test_callback();
                }

                function test2() {
                    test1();
                }

                test2();
            "#,
        )
        .unwrap();
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();

        #[derive(Collect)]
        #[collect(require_static)]
        struct TestHook {
            frame_count: Rc<Cell<u32>>,
            max_count: Rc<Cell<u32>>,
        }

        impl<'gc> vm::Hook<'gc> for TestHook {
            fn on_call(
                &mut self,
                _ctx: vm::Context<'gc>,
                backtrace: vm::Backtrace<'gc, '_>,
            ) -> Result<(), vm::RuntimeError> {
                self.frame_count.set(self.frame_count.get() + 1);
                self.max_count
                    .set(self.max_count.get().max(self.frame_count.get()));

                match backtrace.frame_depth() {
                    1 => assert!(matches!(backtrace.frame(0), vm::BacktraceFrame::Closure(_))),
                    2 => assert!(matches!(backtrace.frame(0), vm::BacktraceFrame::Closure(_))),
                    3 => assert!(matches!(backtrace.frame(0), vm::BacktraceFrame::Closure(_))),
                    4 => assert!(matches!(
                        backtrace.frame(0),
                        vm::BacktraceFrame::Callback(_)
                    )),
                    _ => unreachable!(),
                }
                Ok(())
            }

            fn on_return(&mut self, _ctx: vm::Context<'gc>, backtrace: vm::Backtrace<'gc, '_>) {
                self.frame_count.set(self.frame_count.get() - 1);

                match backtrace.frame_depth() {
                    1 => assert!(matches!(backtrace.frame(0), vm::BacktraceFrame::Closure(_))),
                    2 => assert!(matches!(backtrace.frame(0), vm::BacktraceFrame::Closure(_))),
                    3 => assert!(matches!(backtrace.frame(0), vm::BacktraceFrame::Closure(_))),
                    4 => assert!(matches!(
                        backtrace.frame(0),
                        vm::BacktraceFrame::Callback(_)
                    )),
                    _ => unreachable!(),
                }
            }
        }

        let thread = vm::Thread::new(&ctx);

        let frame_count = Rc::new(Cell::new(0));
        let max_count = Rc::new(Cell::new(0));
        thread.set_hook(
            &ctx,
            TestHook {
                frame_count: frame_count.clone(),
                max_count: max_count.clone(),
            },
        );

        thread.run(ctx, closure).unwrap();

        assert!(frame_count.get() == 0);
        assert!(max_count.get() == 4);
    });
}

#[test]
fn test_vm_return_hook_on_error() {
    let interpreter = vm::Interpreter::new();

    interpreter.enter(|ctx| {
        let output = compiler::Compiler::compile_chunk(
            ctx,
            "default",
            compiler::ImportItems::with_magic(&ctx, ctx.stdlib()),
            compiler::CompileSettings::strict(),
            "vm hook test",
            r#"
                function test1() {
                    throw "hello";
                }

                function test2() {
                    test1();
                }

                test2();
            "#,
        )
        .unwrap();
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();

        #[derive(Collect)]
        #[collect(require_static)]
        struct TestHook {
            frame_count: Rc<Cell<u32>>,
        }

        impl<'gc> vm::Hook<'gc> for TestHook {
            fn on_call(
                &mut self,
                _ctx: vm::Context<'gc>,
                backtrace: vm::Backtrace<'gc, '_>,
            ) -> Result<(), vm::RuntimeError> {
                self.frame_count.set(self.frame_count.get() + 1);
                assert!(self.frame_count.get() as usize == backtrace.frame_depth());
                Ok(())
            }

            fn on_return(&mut self, _ctx: vm::Context<'gc>, backtrace: vm::Backtrace<'gc, '_>) {
                assert!(self.frame_count.get() as usize == backtrace.frame_depth());
                self.frame_count.set(self.frame_count.get() - 1);
            }
        }

        let thread = vm::Thread::new(&ctx);

        let frame_count = Rc::new(Cell::new(0));
        thread.set_hook(
            &ctx,
            TestHook {
                frame_count: frame_count.clone(),
            },
        );

        assert!(matches!(
            thread.run(ctx, closure),
            Err(vm::CallError::Vm {
                error: vm::ExternError::Script(vm::ExternScriptError(vm::ExternValue::String(s))),
                ..
            }) if s.as_str() == "hello"
        ));

        assert!(frame_count.get() == 0);
    });
}

#[test]
fn test_vm_call_return_hook_count_with_error() {
    let interpreter = vm::Interpreter::new();

    interpreter.enter(|ctx| {
        let mut magic = vm::MagicSet::new();
        magic.merge(&ctx.stdlib());

        magic.insert_constant(
            ctx,
            "test_callback",
            vm::Callback::from_fn(&ctx, |_ctx, mut exec| {
                exec.stack().clear();
                Ok(())
            }),
        );

        let magic = Gc::new(&ctx, magic);

        let output = compiler::Compiler::compile_chunk(
            ctx,
            "default",
            compiler::ImportItems::with_magic(&ctx, magic),
            compiler::CompileSettings::strict(),
            "vm hook test",
            r#"
                function test1() {
                    test_callback();
                }

                function test2() {
                    test1();
                }

                test2();
            "#,
        )
        .unwrap();
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();

        #[derive(Debug)]
        struct NoCallbacksAllowed;

        impl fmt::Display for NoCallbacksAllowed {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "no callbacks allowed")
            }
        }

        impl Error for NoCallbacksAllowed {}

        #[derive(Collect)]
        #[collect(require_static)]
        struct TestHook {
            frame_count: Rc<Cell<u32>>,
        }

        impl<'gc> vm::Hook<'gc> for TestHook {
            fn on_call(
                &mut self,
                _ctx: vm::Context<'gc>,
                backtrace: vm::Backtrace<'gc, '_>,
            ) -> Result<(), vm::RuntimeError> {
                self.frame_count.set(self.frame_count.get() + 1);
                if matches!(backtrace.frame(0), vm::BacktraceFrame::Callback(_)) {
                    Err(NoCallbacksAllowed.into())
                } else {
                    Ok(())
                }
            }

            fn on_return(&mut self, _ctx: vm::Context<'gc>, backtrace: vm::Backtrace<'gc, '_>) {
                assert!(self.frame_count.get() as usize == backtrace.frame_depth());
                self.frame_count.set(self.frame_count.get() - 1);
            }
        }

        let thread = vm::Thread::new(&ctx);

        let frame_count = Rc::new(Cell::new(0));
        thread.set_hook(
            &ctx,
            TestHook {
                frame_count: frame_count.clone(),
            },
        );

        assert!(matches!(
            thread.run(ctx, closure),
            Err(vm::CallError::Vm {
                error: vm::ExternError::Runtime(err),
                ..
            }) if err.is::<NoCallbacksAllowed>()
        ));

        assert!(frame_count.get() == 0);
    });
}

#[test]
fn test_vm_step_hook() {
    let interpreter = vm::Interpreter::new();

    interpreter.enter(|ctx| {
        let output = compiler::Compiler::compile_chunk(
            ctx,
            "default",
            compiler::ImportItems::with_magic(&ctx, ctx.stdlib()),
            compiler::CompileSettings::strict(),
            "vm hook test",
            r#"
                function small_loop() {
                    for (let i = 0; i < 100; ++i) {}
                }

                while true {
                    small_loop();
                }
            "#,
        )
        .unwrap();
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();

        #[derive(Debug)]
        struct ExecLimitError;

        impl fmt::Display for ExecLimitError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "exec limit error")
            }
        }

        impl Error for ExecLimitError {}

        #[derive(Collect)]
        #[collect(require_static)]
        struct TestHook(u32);

        impl<'gc> vm::Hook<'gc> for TestHook {
            fn on_step(
                &mut self,
                _ctx: vm::Context<'gc>,
                inst_count: u32,
            ) -> Result<u32, vm::RuntimeError> {
                self.0 = self.0.saturating_sub(inst_count);
                if self.0 == 0 {
                    Err(ExecLimitError.into())
                } else {
                    Ok(self.0)
                }
            }
        }

        let thread = vm::Thread::new(&ctx);
        thread.set_hook(&ctx, TestHook(10_000));

        match thread.run(ctx, closure) {
            Err(vm::CallError::Vm {
                error: vm::ExternError::Runtime(runtime_err),
                ..
            }) => assert!(runtime_err.is::<ExecLimitError>()),
            _ => panic!("should not return without a VM runtime error"),
        }
    });
}
