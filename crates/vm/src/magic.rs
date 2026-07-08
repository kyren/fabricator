use std::{collections::hash_map, fmt};

use gc_arena::{Collect, Gc, Mutation, barrier};
use thiserror::Error;

use crate::{
    builtins::BuiltIns,
    error::RuntimeError,
    interpreter::Context,
    string::{String, StringMap},
    value::Value,
};

#[derive(Debug, Error)]
#[error("cannot write to a read only magic value")]
pub struct MagicReadOnly;

/// A trait for "magic" global variables in FML.
///
/// Magic variables are always available in every scope, and can only be shadowed by a local
/// variable declaration. This means that any time a *free* variable is referenced with the name
/// of a magic variable, it will always refer to the magic variable (and never to, for example,
/// a global).
///
/// Magic variables can be used to provide an API to scripts that is usable no matter the current
/// `self` value, without having to explicitly reference the `global` table.
///
/// Magic variables can optionally be writeable. This does not *replace* the magic value like would
/// occur normally in FML, instead it triggers a write callback for that particular magic value.
pub trait Magic<'gc> {
    fn get(&self, ctx: Context<'gc>) -> Result<Value<'gc>, RuntimeError>;

    fn set(&self, _ctx: Context<'gc>, _value: Value<'gc>) -> Result<(), RuntimeError> {
        Err(MagicReadOnly.into())
    }

    // Magic variable should be treated as read-only, calling `Magic::set` will error.
    fn read_only(&self) -> bool {
        true
    }
}

/// A simple implementation of the `Magic` trait that provides a read-only constant.
#[derive(Collect)]
#[collect(no_drop)]
pub struct MagicConstant<'gc>(Value<'gc>);

impl<'gc> MagicConstant<'gc> {
    pub fn new(value: impl Into<Value<'gc>>) -> Self {
        Self(value.into())
    }

    pub fn new_ptr(mc: &Mutation<'gc>, value: impl Into<Value<'gc>>) -> Gc<'gc, dyn Magic<'gc>> {
        gc_arena::unsize!(Gc::new(mc, Self::new(value)) => dyn Magic)
    }
}

impl<'gc> Magic<'gc> for MagicConstant<'gc> {
    fn get(&self, _ctx: Context<'gc>) -> Result<Value<'gc>, RuntimeError> {
        Ok(self.0)
    }
}

#[derive(Debug, Copy, Clone, Error)]
#[error("no such magic variable with index {0}")]
pub struct BadMagicIndex(pub usize);

/// A set for all magic variable available to some FML script.
///
/// Magic variables are always referenced in the VM by their index for speed, rather than by name.
#[derive(Clone, Default, Collect)]
#[collect(no_drop)]
pub struct MagicSet<'gc> {
    registered: Vec<gc_arena::Lock<Gc<'gc, dyn Magic<'gc>>>>,
    names: StringMap<'gc, usize>,
}

impl<'gc> fmt::Debug for MagicSet<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct MagicSetDebug<'a, 'gc>(&'a StringMap<'gc, usize>);

        impl<'a, 'gc> fmt::Debug for MagicSetDebug<'a, 'gc> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut m = f.debug_map();
                for (name, index) in self.0 {
                    m.entry(&index, &name.as_str());
                }
                m.finish()
            }
        }

        f.debug_tuple("MagicSet")
            .field(&MagicSetDebug(&self.names))
            .finish()
    }
}

impl<'gc> MagicSet<'gc> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builtins(ctx: Context<'gc>) -> Self {
        let mut magic = Self::new();
        BuiltIns::singleton(ctx).insert_builtins(ctx, &mut magic);
        magic
    }

    pub fn is_empty(&self) -> bool {
        self.registered.is_empty()
    }

    /// Add a new or set an existing magic variable in a `MagicSet`.
    ///
    /// If this `MagicSet` did not previously contain a magic variable with this name, then it will
    /// be inserted and this method will return a tuple of the new variable's index and `true`.
    ///
    /// If the `MagicSet` did previously contain a variable with this name, then the previous value
    /// will be overwritten with the given one and this method will return a tuple of the existing
    /// variable's index and `false`.
    pub fn insert(&mut self, name: String<'gc>, value: Gc<'gc, dyn Magic<'gc>>) -> (usize, bool) {
        match self.names.entry(name) {
            hash_map::Entry::Occupied(occupied) => {
                let index = *occupied.get();
                self.registered[index] = gc_arena::Lock::new(value);
                (index, false)
            }
            hash_map::Entry::Vacant(vacant) => {
                let index = self.registered.len();
                vacant.insert(index);
                self.registered.push(gc_arena::Lock::new(value));
                (index, true)
            }
        }
    }

    pub fn names(&self) -> impl Iterator<Item = (String<'gc>, usize)> {
        self.names.iter().map(|(n, i)| (*n, *i))
    }

    /// Merge the given `MagicSet`, overwriting any existing values.
    ///
    /// No existing variable will change its index, even if it is overwritten. All newly inserted
    /// variables will be assigned new indexes.
    pub fn merge(&mut self, other: &MagicSet<'gc>) {
        for (&name, &index) in &other.names {
            self.insert(name, other.registered[index].get());
        }
    }

    /// Find the index for a magic variable with the given name, if it exists.
    pub fn find(&self, name: String<'gc>) -> Option<usize> {
        self.names.get(&name).copied()
    }

    /// Get the magic value associated with the given index.
    pub fn get(&self, index: usize) -> Result<Gc<'gc, dyn Magic<'gc>>, BadMagicIndex> {
        if let Some(var) = self.registered.get(index) {
            Ok(var.get())
        } else {
            Err(BadMagicIndex(index))
        }
    }

    /// Replace the value for an already registered magic variable.
    pub fn replace(
        this: &barrier::Write<MagicSet<'gc>>,
        index: usize,
        value: Gc<'gc, dyn Magic<'gc>>,
    ) -> Result<(), BadMagicIndex> {
        let registered = barrier::field!(this, MagicSet, registered);

        if index >= registered.len() {
            return Err(BadMagicIndex(index));
        }

        registered[index].unlock().set(value);
        Ok(())
    }
}
