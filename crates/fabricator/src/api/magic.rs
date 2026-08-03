use fabricator_vm as vm;
use gc_arena::{Collect, Gc};
use thiserror::Error;

pub fn create_magic_ro<'gc>(
    ctx: vm::Context<'gc>,
    read: impl Fn(vm::Context<'gc>) -> Result<vm::Value<'gc>, vm::RuntimeError> + 'static,
) -> Gc<'gc, dyn vm::Magic<'gc>> {
    #[derive(Collect)]
    #[collect(require_static)]
    struct Magic<R> {
        read: R,
    }

    impl<'gc, R> vm::Magic<'gc> for Magic<R>
    where
        R: Fn(vm::Context<'gc>) -> Result<vm::Value<'gc>, vm::RuntimeError>,
    {
        fn get(&self, ctx: vm::Context<'gc>) -> Result<vm::Value<'gc>, vm::RuntimeError> {
            (self.read)(ctx)
        }
    }

    gc_arena::unsize!(ctx.alloc(Magic { read }) => dyn vm::Magic)
}

pub fn create_magic_rw<'gc>(
    ctx: vm::Context<'gc>,
    read: impl Fn(vm::Context<'gc>) -> Result<vm::Value<'gc>, vm::RuntimeError> + 'static,
    write: impl Fn(vm::Context<'gc>, vm::Value<'gc>) -> Result<(), vm::RuntimeError> + 'static,
) -> Gc<'gc, dyn vm::Magic<'gc>> {
    #[derive(Collect)]
    #[collect(require_static)]
    struct Magic<R, W> {
        read: R,
        write: W,
    }

    impl<'gc, R, W> vm::Magic<'gc> for Magic<R, W>
    where
        R: Fn(vm::Context<'gc>) -> Result<vm::Value<'gc>, vm::RuntimeError>,
        W: Fn(vm::Context<'gc>, vm::Value<'gc>) -> Result<(), vm::RuntimeError>,
    {
        fn get(&self, ctx: vm::Context<'gc>) -> Result<vm::Value<'gc>, vm::RuntimeError> {
            (self.read)(ctx)
        }

        fn set(
            &self,
            ctx: vm::Context<'gc>,
            value: vm::Value<'gc>,
        ) -> Result<(), vm::RuntimeError> {
            (self.write)(ctx, value)
        }

        fn read_only(&self) -> bool {
            false
        }
    }

    gc_arena::unsize!(ctx.alloc(Magic { read, write }) => dyn vm::Magic)
}

#[derive(Debug, Error)]
#[error("magic variable name {0:?} is not unique")]
pub struct DuplicateMagicName(String);

pub trait MagicExt<'gc> {
    /// Add a new magic variable to this `MagicSet`.
    ///
    /// If an existing magic variable already exists with this name, does nothing and returns
    /// `Err(DuplicateMagicName)`
    fn add(
        &mut self,
        name: vm::String<'gc>,
        value: Gc<'gc, dyn vm::Magic<'gc>>,
    ) -> Result<usize, DuplicateMagicName>;

    /// Convenience method to add any value that implements the `Magic` trait.
    fn add_impl<M>(
        &mut self,
        ctx: vm::Context<'gc>,
        name: vm::String<'gc>,
        magic: M,
    ) -> Result<usize, DuplicateMagicName>
    where
        M: vm::Magic<'gc> + Collect<'gc> + 'gc;

    /// Convenience method to add a read-only magic variable which always contains a constant value.
    fn add_constant(
        &mut self,
        ctx: vm::Context<'gc>,
        name: vm::String<'gc>,
        value: impl Into<vm::Value<'gc>>,
    ) -> Result<usize, DuplicateMagicName>;

    /// Merge the given `MagicSet` if and only if every name in it is unique, otherwise, does
    /// nothing and returns `Err(DuplicateMagicName)`
    ///
    /// All existing variables will not change their index, all merged variables will be assigned
    /// new indexes.
    fn merge_unique(&mut self, other: &vm::MagicSet<'gc>) -> Result<(), DuplicateMagicName>;
}

impl<'gc> MagicExt<'gc> for vm::MagicSet<'gc> {
    fn add(
        &mut self,
        name: vm::String<'gc>,
        value: Gc<'gc, dyn vm::Magic<'gc>>,
    ) -> Result<usize, DuplicateMagicName> {
        if self.find(name).is_some() {
            return Err(DuplicateMagicName(name.as_str().to_owned()));
        }

        let (index, _) = self.insert(name, value);
        Ok(index)
    }

    fn add_impl<M>(
        &mut self,
        ctx: vm::Context<'gc>,
        name: vm::String<'gc>,
        magic: M,
    ) -> Result<usize, DuplicateMagicName>
    where
        M: vm::Magic<'gc> + Collect<'gc> + 'gc,
    {
        self.add(name, gc_arena::unsize!(ctx.alloc(magic) => dyn vm::Magic))
    }

    fn add_constant(
        &mut self,
        ctx: vm::Context<'gc>,
        name: vm::String<'gc>,
        value: impl Into<vm::Value<'gc>>,
    ) -> Result<usize, DuplicateMagicName> {
        self.add(name, vm::MagicConstant::new_ptr(&ctx, value.into()))
    }

    fn merge_unique(&mut self, other: &vm::MagicSet<'gc>) -> Result<(), DuplicateMagicName> {
        for (name, _) in other.names() {
            if self.find(name).is_some() {
                return Err(DuplicateMagicName(name.as_str().to_owned()));
            }
        }

        for (name, index) in other.names() {
            self.add(name, other.get(index).unwrap()).unwrap();
        }
        Ok(())
    }
}
