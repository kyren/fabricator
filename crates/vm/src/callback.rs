use std::{fmt, hash};

use gc_arena::{Collect, Gc, Mutation};

use crate::{error::RuntimeError, interpreter::Context, thread::Execution, value::Value};

pub trait CallbackFn<'gc> {
    fn call(&self, ctx: Context<'gc>, exec: Execution<'gc, '_>) -> Result<(), RuntimeError>;
}

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct CallbackInner<'gc> {
    callback_fn: Gc<'gc, dyn CallbackFn<'gc>>,
    this: Value<'gc>,
}

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct Callback<'gc>(Gc<'gc, CallbackInner<'gc>>);

impl<'gc> Callback<'gc> {
    pub fn new<C: CallbackFn<'gc> + Collect<'gc> + 'gc>(
        mc: &Mutation<'gc>,
        callback: C,
        this: Value<'gc>,
    ) -> Self {
        let callback_fn = gc_arena::unsize!(Gc::new(mc, callback) => dyn CallbackFn);
        Self(Gc::new(mc, CallbackInner { callback_fn, this }))
    }

    /// Call the contained callback.
    ///
    /// If there is a `self` object bound to the callback, then the provided `exec` will be rebound
    /// with it.
    #[inline]
    pub fn call(self, ctx: Context<'gc>, mut exec: Execution<'gc, '_>) -> Result<(), RuntimeError> {
        self.0.callback_fn.call(
            ctx,
            if self.0.this.is_undefined() {
                exec
            } else {
                exec.with_this(self.0.this)
            },
        )
    }

    /// Return a clone of this callback with the embedded `self` value changed to the provided one.
    ///
    /// If `Value::Undefined` is provided, then the bound `self` object will be removed.
    #[inline]
    pub fn rebind(self, mc: &Mutation<'gc>, this: Value<'gc>) -> Callback<'gc> {
        Self(Gc::new(
            mc,
            CallbackInner {
                callback_fn: self.0.callback_fn,
                this,
            },
        ))
    }

    /// Returns the currently bound `self` object, or `Value::Undefined` if one is not set.
    #[inline]
    pub fn this(self) -> Value<'gc> {
        self.0.this
    }

    /// Create a callback from a Rust function.
    ///
    /// The function must be `'static` because Rust closures cannot implement `Collect`. If you need
    /// to associate GC data with this function, use [`Callback::from_fn_with_root`].
    pub fn from_fn<F>(mc: &Mutation<'gc>, call: F) -> Callback<'gc>
    where
        F: 'static + Fn(Context<'gc>, Execution<'gc, '_>) -> Result<(), RuntimeError>,
    {
        Self::from_fn_with_root(mc, (), move |_, ctx, exec| call(ctx, exec))
    }

    /// Create a callback from a Rust function together with a GC object.
    pub fn from_fn_with_root<R, F>(mc: &Mutation<'gc>, root: R, call: F) -> Callback<'gc>
    where
        R: 'gc + Collect<'gc>,
        F: 'static + Fn(&R, Context<'gc>, Execution<'gc, '_>) -> Result<(), RuntimeError>,
    {
        #[derive(Collect)]
        #[collect(no_drop)]
        struct RootCallback<R, F> {
            root: R,
            #[collect(require_static)]
            call: F,
        }

        impl<'gc, R, F> CallbackFn<'gc> for RootCallback<R, F>
        where
            R: 'gc + Collect<'gc>,
            F: 'static + Fn(&R, Context<'gc>, Execution<'gc, '_>) -> Result<(), RuntimeError>,
        {
            fn call(
                &self,
                ctx: Context<'gc>,
                exec: Execution<'gc, '_>,
            ) -> Result<(), RuntimeError> {
                (self.call)(&self.root, ctx, exec)
            }
        }

        Callback::new(mc, RootCallback { root, call }, Value::Undefined)
    }

    #[inline]
    pub fn from_inner(inner: Gc<'gc, CallbackInner<'gc>>) -> Self {
        Self(inner)
    }

    #[inline]
    pub fn into_inner(self) -> Gc<'gc, CallbackInner<'gc>> {
        self.0
    }
}

impl<'gc> fmt::Debug for Callback<'gc> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_tuple("Callback")
            .field(&Gc::as_ptr(self.0))
            .finish()
    }
}

impl<'gc> PartialEq for Callback<'gc> {
    fn eq(&self, other: &Callback<'gc>) -> bool {
        Gc::ptr_eq(self.0, other.0)
    }
}

impl<'gc> Eq for Callback<'gc> {}

impl<'gc> hash::Hash for Callback<'gc> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        Gc::as_ptr(self.0).hash(state)
    }
}
