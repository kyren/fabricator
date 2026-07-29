use std::convert::Infallible;

use fabricator_stdlib::{StdlibContext as _, util::MagicExt};
use fabricator_vm as vm;
use gc_arena::{Collect, Gc, Rootable};

pub trait TestingStdlibContext<'gc> {
    /// The stdlib with some additional test methods.
    fn testing_stdlib(self) -> Gc<'gc, vm::MagicSet<'gc>>;
}

impl<'gc> TestingStdlibContext<'gc> for vm::Context<'gc> {
    fn testing_stdlib(self) -> Gc<'gc, vm::MagicSet<'gc>> {
        #[derive(Collect)]
        #[collect(no_drop)]
        struct TestingStdlibSingleton<'gc>(Gc<'gc, vm::MagicSet<'gc>>);

        impl<'gc> vm::Singleton<'gc> for TestingStdlibSingleton<'gc> {
            fn create(ctx: vm::Context<'gc>) -> Self {
                let mut lib = vm::MagicSet::new();
                lib.merge(&ctx.stdlib());

                lib.insert_exec_callback(ctx, "assert", |_, mut exec| {
                    let stack = exec.stack();
                    for i in 0..stack.len() {
                        if !stack.get(i).cast_bool() {
                            return Err(vm::RuntimeError::msg(format!("assert {i} failed")));
                        }
                    }
                    Ok(())
                });

                lib.insert_exec_callback(ctx, "black_box", |_, _| Ok::<_, Infallible>(()));

                TestingStdlibSingleton(Gc::new(&ctx, lib))
            }
        }

        self.singleton::<Rootable![TestingStdlibSingleton<'_>]>().0
    }
}
