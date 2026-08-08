use std::ops;

use gc_arena::{
    Arena, Collect, Gc, Mutation, Rootable,
    arena::{CollectionPhase as GcCollectionPhase, Root},
    metrics::Metrics as GcMetrics,
    zst_cache::ZstCache,
};

use crate::{
    object::Object,
    registry::{Registry, Singleton},
    stash::{Fetchable, Stashable},
    string::{InternedStrings, String},
};

pub const MAX_ZST_CACHE_ALIGN: usize = 16;

#[derive(Copy, Clone)]
pub struct Context<'gc> {
    mutation: &'gc Mutation<'gc>,
    state: &'gc State<'gc>,
}

impl<'gc> ops::Deref for Context<'gc> {
    type Target = Mutation<'gc>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.mutation
    }
}

impl<'gc> Context<'gc> {
    /// Get a reference to [`Mutation`] (the `gc-arena` mutation handle) out of the `Context`
    /// object.
    ///
    /// This can also be done automatically with [`ops::Deref`] coercion.
    #[inline]
    pub fn mutation(self) -> &'gc Mutation<'gc> {
        self.mutation
    }

    #[inline]
    pub fn globals(self) -> Object<'gc> {
        self.state.globals
    }

    #[inline]
    pub fn registry(self) -> Registry<'gc> {
        self.state.registry
    }

    /// Calls `ctx.registry().singleton::<S>(ctx)`.
    #[inline]
    pub fn singleton<S>(self) -> &'gc Root<'gc, S>
    where
        S: for<'a> Rootable<'a> + 'static,
        Root<'gc, S>: Sized + Singleton<'gc> + Collect<'gc>,
    {
        self.state.registry.singleton::<S>(self)
    }

    /// Calls `ctx.registry().stash(ctx, s)`.
    #[inline]
    pub fn stash<S: Stashable<'gc>>(self, s: S) -> S::Stashed {
        self.state.registry.stash(&self, s)
    }

    /// Calls `ctx.registry().fetch(f)`.
    #[inline]
    pub fn fetch<F: Fetchable + Fetchable>(self, f: &F) -> F::Fetched<'gc> {
        self.state.registry.fetch(f)
    }

    #[inline]
    pub fn interned_strings(self) -> InternedStrings<'gc> {
        self.state.interned_strings
    }

    #[inline]
    pub fn intern(self, s: &str) -> String<'gc> {
        self.state.interned_strings.intern(&self, s, || s.into())
    }

    pub fn zst_cache(self) -> ZstCache<'gc, MAX_ZST_CACHE_ALIGN> {
        self.state.zst_cache
    }

    /// Allocate a ZST using the cached ZST pointer.
    ///
    /// Returns `None` if the type is not a ZST or has alignment greater than
    /// [`MAX_ZST_CACHE_ALIGN`].
    pub fn alloc_zst<T: 'gc>(self) -> Option<Gc<'gc, T>> {
        self.state.zst_cache.alloc_zst()
    }

    /// This method is the same as [`Gc::new`], except that it will automatically use a shared
    /// allocation for ZST types.
    pub fn alloc<T: Collect<'gc>>(self, t: T) -> Gc<'gc, T> {
        self.state.zst_cache.alloc(self.mutation, t)
    }

    /// This method is the same as [`Gc::new_static`], except that it will automatically use a
    /// shared allocation for ZST types.
    pub fn alloc_static<T: 'static>(self, t: T) -> Gc<'gc, T> {
        self.state.zst_cache.alloc_static(self.mutation, t)
    }
}

pub struct Interpreter {
    arena: Arena<Rootable![State<'_>]>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            arena: Arena::<Rootable![State<'_>]>::new(|mc| State::new(mc)),
        }
    }

    /// Enter the interpreter context.
    pub fn enter<F, T>(&self, f: F) -> T
    where
        F: for<'gc> FnOnce(Context<'gc>) -> T,
    {
        self.arena.mutate(move |mc, state| f(state.ctx(mc)))
    }

    pub fn gc_metrics(&self) -> &GcMetrics {
        self.arena.metrics()
    }

    /// Collect any outstanding GC debt according to the configured GC pacing.
    pub fn gc_collect_debt(&mut self) {
        self.arena.collect_debt();
    }

    /// Finish the current GC cycle
    pub fn gc_finish_cycle(&mut self) {
        self.arena.finish_cycle();
    }

    pub fn gc_collection_phase(&self) -> GcCollectionPhase {
        self.arena.collection_phase()
    }
}

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
struct State<'gc> {
    globals: Object<'gc>,
    registry: Registry<'gc>,
    interned_strings: InternedStrings<'gc>,
    zst_cache: ZstCache<'gc, MAX_ZST_CACHE_ALIGN>,
}

impl<'gc> State<'gc> {
    fn new(mc: &Mutation<'gc>) -> State<'gc> {
        Self {
            globals: Object::new(mc),
            registry: Registry::new(mc),
            interned_strings: InternedStrings::new(mc),
            zst_cache: ZstCache::new(mc),
        }
    }

    fn ctx(&'gc self, mutation: &'gc Mutation<'gc>) -> Context<'gc> {
        Context {
            mutation,
            state: self,
        }
    }
}
