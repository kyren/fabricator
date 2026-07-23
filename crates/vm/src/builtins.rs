use gc_arena::{Collect, Mutation, Rootable};

use crate::{
    callback::Callback,
    closure::Closure,
    error::RuntimeError,
    interpreter::Context,
    magic::{MagicConstant, MagicSet},
    object::Object,
    registry::Singleton,
    thread::IndexError,
    user_data::UserDataIter,
    value::{Function, Value},
};

/// FML values for core VM functionality.
///
/// Will be assumed to be present in any `MagicSet`, and may be required for compilation.
#[derive(Collect)]
#[collect(no_drop)]
pub struct BuiltIns<'gc> {
    /// Rebind the implicit `self` on a callback or closure.
    ///
    /// ```fml
    /// let f = function() {
    ///     return self.field;
    /// };
    ///
    /// let t = {
    ///     field: true,
    /// };
    ///
    /// let f_rebound = bind(t, f);
    /// ```
    pub bind: Callback<'gc>,

    /// Call the given function and catch any errors.
    ///
    /// The first parameter is the function to call, the rest are parameters to pass to the provided
    /// function.
    ///
    /// If the given function completes without error, returns `true` followed by the return values
    /// of the inner function.
    ///
    /// If there is an error executing the given function, returns `false` followed by the error.
    ///
    /// ```fml
    /// let success, err = pcall(function() {
    ///     throw "my_error";
    /// });
    ///
    /// assert(success == false);
    /// assert(err == "my_error");
    /// ```
    pub pcall: Callback<'gc>,

    /// Get the parent (super) of an object if it exists.
    pub get_super: Callback<'gc>,

    /// Give an object a new parent (super).
    ///
    /// ```fml
    /// let obj = {
    ///     a: 1,
    /// };
    ///
    /// let parent = {
    ///     b: 2,
    /// };
    ///
    /// super_set(obj, parent);
    ///
    /// assert(obj.a == 1);
    /// assert(obj.b == 2);
    /// ```
    pub set_super: Callback<'gc>,

    /// Get the constructor super object for the prototype of the given closure, initializing it if
    /// it is not yet initialized.
    ///
    /// This is an internal compiler support method.
    pub init_constructor_super: Callback<'gc>,

    /// Get the constructor super object for the prototype of the given closure, if it has been
    /// initialized.
    ///
    /// This is an internal compiler support method.
    pub get_constructor_super: Callback<'gc>,

    /// Return the loop function and initial state for a `with` loop on the given object.
    ///
    /// This is an internal compiler support method.
    pub with_loop_iter: Callback<'gc>,

    /// Get the value at the given index, potentially with multiple index values.
    ///
    /// The first parameter is the target and all subsequent parameters are the indexes.
    ///
    /// This is an internal compiler support method.
    pub get_multi_index: Callback<'gc>,

    /// Set the value at the given index, potentially with multiple index values.
    ///
    /// The first parameter is the target, the second is the value to set, and all subsequent
    /// parameters are the indexes.
    ///
    /// This is an internal compiler support method.
    pub set_multi_index: Callback<'gc>,
}

impl<'gc> BuiltIns<'gc> {
    pub const BIND: &'static str = "bind";

    pub const PCALL: &'static str = "pcall";

    pub const GET_SUPER: &'static str = "get_super";
    pub const SET_SUPER: &'static str = "set_super";

    pub const INIT_CONSTRUCTOR_SUPER: &'static str = "__init_constructor_super";
    pub const GET_CONSTRUCTOR_SUPER: &'static str = "__get_constructor_super";

    pub const WITH_LOOP_ITER: &'static str = "__with_loop_iter";

    pub const GET_MULTI_INDEX: &'static str = "__get_multi_index";
    pub const SET_MULTI_INDEX: &'static str = "__set_multi_index";

    fn new(mc: &Mutation<'gc>) -> Self {
        Self {
            bind: Callback::from_fn(mc, |ctx, mut exec| {
                let (obj, func): (Value, Function) = exec.stack().consume(ctx)?;

                match obj {
                    obj @ (Value::Undefined | Value::Object(_) | Value::UserData(_)) => {
                        exec.stack().replace(ctx, func.rebind(&ctx, obj));
                        Ok(())
                    }
                    _ => Err(RuntimeError::msg(
                        "self value must be an object, userdata, or undefined",
                    )),
                }
            }),

            pcall: Callback::from_fn(mc, |ctx, mut exec| {
                let function: Function = exec.stack().from_index(ctx, 0)?;
                let res = {
                    let mut sub_exec = exec.with_stack_bottom(1);
                    match function {
                        Function::Closure(closure) => {
                            sub_exec.call_closure(ctx, closure).map_err(|e| e.error)
                        }
                        Function::Callback(callback) => {
                            sub_exec.call_callback(ctx, callback).map_err(|e| e.into())
                        }
                    }
                };
                match res {
                    Ok(_) => {
                        exec.stack()[0] = true.into();
                    }
                    Err(err) => {
                        exec.stack().replace(ctx, (false, err.to_value(ctx)));
                    }
                }
                Ok(())
            }),

            get_super: Callback::from_fn(mc, |ctx, mut exec| {
                let obj: Object = exec.stack().consume(ctx)?;
                exec.stack().replace(ctx, obj.parent());
                Ok(())
            }),

            set_super: Callback::from_fn(mc, |ctx, mut exec| {
                let (obj, parent): (Object, Option<Object>) = exec.stack().consume(ctx)?;
                obj.set_parent(&ctx, parent)?;
                exec.stack().replace(ctx, obj);
                Ok(())
            }),

            init_constructor_super: Callback::from_fn(mc, |ctx, mut exec| {
                let closure: Closure = exec.stack().consume(ctx)?;
                exec.stack()
                    .replace(ctx, closure.prototype().init_constructor_super(&ctx));
                Ok(())
            }),

            get_constructor_super: Callback::from_fn(mc, |ctx, mut exec| {
                let closure: Closure = exec.stack().consume(ctx)?;
                exec.stack()
                    .replace(ctx, closure.prototype().constructor_super());
                Ok(())
            }),

            with_loop_iter: {
                // An iterator function whose state is the single value for iteration and the
                // control variable is expected to be `true` on the first iteration and `false`
                // afterwards.
                let singleton_iter = Callback::from_fn(mc, |_, mut exec| {
                    let state = exec.stack().get(0);
                    let yield_state = exec.stack().get(1).cast_bool();
                    exec.stack().clear();
                    if yield_state {
                        exec.stack().extend([Value::Boolean(false), state]);
                    }
                    Ok(())
                });

                Callback::from_fn_with_root(mc, singleton_iter, |&singleton_iter, ctx, mut exec| {
                    let target: Value = exec.stack().consume(ctx)?;
                    match target {
                        Value::Object(object) => {
                            // Objects are a loop with one iteration over the object itself.
                            exec.stack().push_back(singleton_iter);
                            exec.stack().push_back(object);
                            exec.stack().push_back(Value::Boolean(true));
                            Ok(())
                        }
                        Value::UserData(user_data) => {
                            match user_data.iter(ctx)? {
                                UserDataIter::Singleton => {
                                    // Singleton userdata are a loop with one iteration over the
                                    // userdata itself.
                                    exec.stack().push_back(singleton_iter);
                                    exec.stack().push_back(user_data);
                                    exec.stack().push_back(Value::Boolean(true));
                                }
                                UserDataIter::Iter {
                                    iter,
                                    state,
                                    control,
                                } => {
                                    exec.stack().replace(ctx, (iter, state, control));
                                }
                            }
                            Ok(())
                        }
                        _ => Err(RuntimeError::msg(
                            "with loop target must be object or userdata",
                        )),
                    }
                })
            },

            get_multi_index: Callback::from_fn(mc, |ctx, mut exec| {
                let mut stack = exec.stack();

                let (target, indexes) = match &*stack {
                    [] => (Value::Undefined, [].as_slice()),
                    [target, indexes @ ..] => (*target, indexes),
                };

                let value = match target {
                    Value::Object(target) => {
                        if indexes.len() == 1 {
                            let index = indexes[0];
                            if let Some(index) = index.coerce_string(ctx) {
                                target.get(index).unwrap_or_default()
                            } else {
                                return Err(IndexError::BadIndex {
                                    target: target.into(),
                                    index: index.into(),
                                }
                                .into());
                            }
                        } else {
                            return Err(IndexError::BadMultiIndex {
                                target: target.into(),
                                len: indexes.len(),
                            }
                            .into());
                        }
                    }
                    Value::Array(target) => {
                        if indexes.len() == 1 {
                            let index = indexes[0];
                            if let Some(index) =
                                index.cast_integer().and_then(|i| i.try_into().ok())
                            {
                                target.get(index).unwrap_or_default()
                            } else {
                                return Err(IndexError::BadIndex {
                                    target: target.into(),
                                    index: index.into(),
                                }
                                .into());
                            }
                        } else {
                            return Err(IndexError::BadMultiIndex {
                                target: target.into(),
                                len: indexes.len(),
                            }
                            .into());
                        }
                    }
                    Value::UserData(user_data) => user_data.get_index(ctx, indexes)?,
                    target => {
                        return Err(IndexError::NotIndexable {
                            target: target.into(),
                        }
                        .into());
                    }
                };

                stack.replace(ctx, value);
                Ok(())
            }),

            set_multi_index: Callback::from_fn(mc, |ctx, mut exec| {
                let mut stack = exec.stack();

                let (target, value, indexes) = match &*stack {
                    [] => (Value::Undefined, Value::Undefined, [].as_slice()),
                    [target] => (*target, Value::Undefined, [].as_slice()),
                    [target, value, indexes @ ..] => (*target, *value, indexes),
                };

                match target {
                    Value::Object(target) => {
                        if indexes.len() == 1 {
                            let index = indexes[0];
                            if let Some(index) = index.coerce_string(ctx) {
                                target.set(&ctx, index, value);
                            } else {
                                return Err(IndexError::BadIndex {
                                    target: target.into(),
                                    index: index.into(),
                                }
                                .into());
                            }
                        } else {
                            return Err(IndexError::BadMultiIndex {
                                target: target.into(),
                                len: indexes.len(),
                            }
                            .into());
                        }
                    }
                    Value::Array(target) => {
                        if indexes.len() == 1 {
                            let index = indexes[0];
                            if let Some(index) =
                                index.cast_integer().and_then(|i| i.try_into().ok())
                            {
                                target.set(&ctx, index, value);
                            } else {
                                return Err(IndexError::BadIndex {
                                    target: target.into(),
                                    index: index.into(),
                                }
                                .into());
                            }
                        } else {
                            return Err(IndexError::BadMultiIndex {
                                target: target.into(),
                                len: indexes.len(),
                            }
                            .into());
                        }
                    }
                    Value::UserData(user_data) => {
                        user_data.set_index(ctx, indexes, value)?;
                    }
                    target => {
                        return Err(IndexError::NotIndexable {
                            target: target.into(),
                        }
                        .into());
                    }
                }

                stack.clear();
                Ok(())
            }),
        }
    }

    pub fn singleton(ctx: Context<'gc>) -> &'gc BuiltIns<'gc> {
        ctx.singleton::<Rootable![BuiltIns<'_>]>()
    }

    /// Insert all builtins into a `MagicSet`.
    ///
    /// All magic names are string constants available in [`BuiltIns`].
    pub fn insert_builtins(&self, ctx: Context<'gc>, magic_set: &mut MagicSet<'gc>) {
        magic_set.insert(
            ctx.intern(Self::BIND),
            MagicConstant::new_ptr(&ctx, self.bind),
        );

        magic_set.insert(
            ctx.intern(Self::PCALL),
            MagicConstant::new_ptr(&ctx, self.pcall),
        );

        magic_set.insert(
            ctx.intern(Self::GET_SUPER),
            MagicConstant::new_ptr(&ctx, self.get_super),
        );

        magic_set.insert(
            ctx.intern(Self::SET_SUPER),
            MagicConstant::new_ptr(&ctx, self.set_super),
        );

        magic_set.insert(
            ctx.intern(Self::INIT_CONSTRUCTOR_SUPER),
            MagicConstant::new_ptr(&ctx, self.init_constructor_super),
        );

        magic_set.insert(
            ctx.intern(Self::GET_CONSTRUCTOR_SUPER),
            MagicConstant::new_ptr(&ctx, self.get_constructor_super),
        );

        magic_set.insert(
            ctx.intern(Self::WITH_LOOP_ITER),
            MagicConstant::new_ptr(&ctx, self.with_loop_iter),
        );

        magic_set.insert(
            ctx.intern(Self::GET_MULTI_INDEX),
            MagicConstant::new_ptr(&ctx, self.get_multi_index),
        );

        magic_set.insert(
            ctx.intern(Self::SET_MULTI_INDEX),
            MagicConstant::new_ptr(&ctx, self.set_multi_index),
        );
    }
}

impl<'gc> Singleton<'gc> for BuiltIns<'gc> {
    fn create(ctx: Context<'gc>) -> Self {
        BuiltIns::new(&ctx)
    }
}
