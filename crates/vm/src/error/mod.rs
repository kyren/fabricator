mod raw_gc;
mod runtime_error;

use std::{error::Error as StdError, fmt};

use gc_arena::{Collect, Gc, GcWeak, Lock, Mutation, Rootable, barrier};
use thiserror::Error;

use crate::{
    interpreter::Context,
    registry::Singleton,
    string::SharedStr,
    string::String,
    user_data::{BadUserDataType, UserData, UserDataMethods},
    value::Value,
};

pub use self::{
    raw_gc::RawGc,
    runtime_error::{RuntimeError, SharedError, ThinArcError},
};

/// An error raised directly from FML which contains a `Value`.
///
/// Any [`Value`] can be raised as an error and it will be contained here.
#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct ScriptError<'gc>(pub Value<'gc>);

impl<'gc> fmt::Display for ScriptError<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl<'gc> From<Value<'gc>> for ScriptError<'gc> {
    fn from(error: Value<'gc>) -> Self {
        ScriptError(error)
    }
}

impl<'gc> ScriptError<'gc> {
    pub fn new(value: Value<'gc>) -> Self {
        Self(value)
    }

    pub fn to_value(self) -> Value<'gc> {
        self.0
    }

    pub fn to_extern(self) -> ExternScriptError {
        self.into()
    }
}

/// An external representation of a [`Value`], useful for errors.
///
/// All primitive values (undefined, booleans, integers, floats) are represented here exactly.
/// Strings are cheaply cloned from an internal shared string. All other Gc types are stored as
/// `RawGc`.
#[derive(Clone)]
pub enum ExternValue {
    Undefined,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(SharedStr),
    Object(RawGc),
    Array(RawGc),
    Closure(RawGc),
    Callback(RawGc),
    UserData(RawGc),
}

impl fmt::Debug for ExternValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternValue::Undefined => write!(f, "`undefined`"),
            ExternValue::Boolean(b) => write!(f, "`{b}`"),
            ExternValue::Integer(i) => write!(f, "`{i}`"),
            ExternValue::Float(n) => write!(f, "`{n}`"),
            ExternValue::String(s) => write!(f, "`{s:?}`"),
            ExternValue::Object(object) => write!(f, "<object {object}>"),
            ExternValue::Array(array) => write!(f, "<array {array}>"),
            ExternValue::Closure(closure) => {
                write!(f, "<closure {closure}>")
            }
            ExternValue::Callback(callback) => {
                write!(f, "<callback {callback}>")
            }
            ExternValue::UserData(user_data) => {
                write!(f, "<user_data {user_data}>")
            }
        }
    }
}

impl fmt::Display for ExternValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternValue::Undefined => write!(f, "undefined"),
            ExternValue::Boolean(b) => write!(f, "{b}"),
            ExternValue::Integer(i) => write!(f, "{i}"),
            ExternValue::Float(n) => write!(f, "{n}"),
            ExternValue::String(s) => write!(f, "{s:?}"),
            ExternValue::Object(object) => {
                write!(f, "<object {object}>")
            }
            ExternValue::Array(array) => write!(f, "<array {array}>"),
            ExternValue::Closure(closure) => {
                write!(f, "<closure {closure}>")
            }
            ExternValue::Callback(callback) => {
                write!(f, "<callback {callback}>")
            }
            ExternValue::UserData(user_data) => {
                write!(f, "<user_data {user_data}>")
            }
        }
    }
}

impl<'gc> From<Value<'gc>> for ExternValue {
    fn from(value: Value<'gc>) -> Self {
        match value {
            Value::Undefined => ExternValue::Undefined,
            Value::Boolean(b) => ExternValue::Boolean(b),
            Value::Integer(i) => ExternValue::Integer(i),
            Value::Float(n) => ExternValue::Float(n),
            Value::String(s) => ExternValue::String(s.as_shared().clone()),
            Value::Object(o) => ExternValue::Object(RawGc::new(o.into_inner())),
            Value::Array(a) => ExternValue::Array(RawGc::new(a.into_inner())),
            Value::Closure(c) => ExternValue::Closure(RawGc::new(c.into_inner())),
            Value::Callback(c) => ExternValue::Callback(RawGc::new(c.into_inner())),
            Value::UserData(u) => ExternValue::UserData(RawGc::new(u.into_inner())),
        }
    }
}

/// A [`ScriptError`] that is not bound to the GC context and holds an [`ExternValue`].
#[derive(Debug, Clone, Error)]
#[error("{0:?}")]
pub struct ExternScriptError(pub ExternValue);

impl<'gc> From<ScriptError<'gc>> for ExternScriptError {
    fn from(error: ScriptError<'gc>) -> Self {
        ExternScriptError(error.to_value().into())
    }
}

/// Any error that can be raised from executing a script.
///
/// This can be either a [`ScriptError`] containing a [`Value`], or a [`RuntimeError`] containing a
/// Rust error.
#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub enum Error<'gc> {
    Script(ScriptError<'gc>),
    Runtime(RuntimeError),
}

impl<'gc> fmt::Display for Error<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Script(err) => write!(f, "script error: {err}"),
            Error::Runtime(err) => write!(f, "runtime error: {err}"),
        }
    }
}

impl<'gc> From<Value<'gc>> for Error<'gc> {
    fn from(value: Value<'gc>) -> Self {
        Self::from_value(value)
    }
}

impl<'gc> From<ScriptError<'gc>> for Error<'gc> {
    fn from(error: ScriptError<'gc>) -> Self {
        Self::Script(error)
    }
}

impl<'gc> From<RuntimeError> for Error<'gc> {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl<'gc, E: StdError + Send + Sync + 'static> From<E> for Error<'gc> {
    fn from(err: E) -> Self {
        Self::Runtime(RuntimeError::new(err))
    }
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
struct RuntimeErrorUserData<'gc> {
    error: RuntimeError,
    // Cache the string representation of this error
    display_cache: Lock<Option<GcWeak<'gc, SharedStr>>>,
}

impl<'gc> RuntimeErrorUserData<'gc> {
    fn new(error: RuntimeError) -> Self {
        Self {
            error,
            display_cache: Lock::new(None),
        }
    }

    fn into_userdata(self, ctx: Context<'gc>) -> UserData<'gc> {
        #[derive(Copy, Clone, Collect)]
        #[collect(require_static)]
        struct Methods;

        impl<'gc> UserDataMethods<'gc> for Methods {
            fn coerce_string(&self, ud: UserData<'gc>, ctx: Context<'gc>) -> Option<String<'gc>> {
                if let Some(s) = RuntimeErrorUserData::downcast(ud)
                    .unwrap()
                    .display_cache
                    .get()
                    .and_then(|s| s.upgrade(&ctx))
                {
                    return Some(String::from_inner(s));
                }

                let this = RuntimeErrorUserData::downcast_write(&ctx, ud).unwrap();

                let string_repr = barrier::field!(this, RuntimeErrorUserData, display_cache);
                let s = ctx.intern(&this.error.to_string());
                string_repr
                    .unlock()
                    .set(Some(Gc::downgrade(String::into_inner(s))));
                Some(s)
            }
        }

        #[derive(Collect)]
        #[collect(no_drop)]
        struct ErrorMethodsSingleton<'gc>(Gc<'gc, dyn UserDataMethods<'gc>>);

        impl<'gc> Singleton<'gc> for ErrorMethodsSingleton<'gc> {
            fn create(ctx: Context<'gc>) -> Self {
                let methods = Gc::new(&ctx, Methods);
                ErrorMethodsSingleton(gc_arena::unsize!(methods => dyn UserDataMethods<'gc>))
            }
        }

        let methods = ctx.singleton::<Rootable![ErrorMethodsSingleton<'_>]>().0;
        let ud = UserData::new::<Rootable![RuntimeErrorUserData<'_>]>(&ctx, self);
        ud.set_methods(&ctx, Some(methods));
        ud.into()
    }

    #[inline]
    fn downcast(ud: UserData<'gc>) -> Result<&'gc RuntimeErrorUserData<'gc>, BadUserDataType> {
        ud.downcast::<Rootable![RuntimeErrorUserData<'_>]>()
    }

    #[inline]
    fn downcast_write(
        mc: &Mutation<'gc>,
        ud: UserData<'gc>,
    ) -> Result<&'gc barrier::Write<RuntimeErrorUserData<'gc>>, BadUserDataType> {
        ud.downcast_write::<Rootable![RuntimeErrorUserData<'_>]>(mc)
    }
}

impl<'gc> Error<'gc> {
    /// Turn a [`Value`] into an `Error`.
    ///
    /// If the provided value is a [`UserData`] object returned from `[Error::to_value]`, then
    /// this conversion will clone the `RuntimeError` held in the `UserData` and properly return
    /// an [`Error::Runtime`] variant. This is how Rust errors are properly transported through
    /// scripts: a `RuntimeError` which is turned into a `Value` with [`Error::to_value`] will
    /// always turn back into a `RuntimeError` error with [`Error::from_value`].
    ///
    /// If the given value is *any other* kind of script value, then this will return a
    /// [`ScriptError`] instead.
    pub fn from_value(value: Value<'gc>) -> Self {
        if let Value::UserData(ud) = value {
            if let Ok(err_ud) = RuntimeErrorUserData::downcast(ud) {
                return Error::Runtime(err_ud.error.clone());
            }
        }

        Error::Script(value.into())
    }

    /// Convert an `Error` into a script value.
    ///
    /// For script errors, this simply returns the original [`Value`] directly.
    ///
    /// For Rust errors, this will return a special [`UserData`] value which holds the
    /// [`RuntimeError`].
    ///
    /// Note that the returned `UserData` is *not the same* as `UserData::new_static(runtime_err)`,
    /// it is impossible to construct this `UserData` in any other way than by calling this method.
    pub fn to_value(&self, ctx: Context<'gc>) -> Value<'gc> {
        match self {
            Error::Script(err) => err.0,
            Error::Runtime(err) => RuntimeErrorUserData::new(err.clone())
                .into_userdata(ctx)
                .into(),
        }
    }

    pub fn into_extern(self) -> ExternError {
        match self {
            Error::Script(script_error) => ExternError::Script(script_error.to_extern()),
            Error::Runtime(runtime_error) => ExternError::Runtime(runtime_error),
        }
    }
}

/// An [`enum@Error`] that is not bound to the GC context.
#[derive(Debug, Clone, Error)]
pub enum ExternError {
    #[error("script error: {0}")]
    Script(#[source] ExternScriptError),
    #[error("runtime error: {0}")]
    Runtime(#[source] RuntimeError),
}

impl From<ExternScriptError> for ExternError {
    fn from(error: ExternScriptError) -> Self {
        Self::Script(error)
    }
}

impl From<RuntimeError> for ExternError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl<'gc> From<Error<'gc>> for ExternError {
    fn from(err: Error<'gc>) -> Self {
        match err {
            Error::Script(err) => err.to_extern().into(),
            Error::Runtime(e) => e.into(),
        }
    }
}
