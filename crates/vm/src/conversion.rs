use std::{array, borrow::Cow, iter, ops, string::String as StdString};

use thiserror::Error;

use crate::{
    array::Array,
    callback::Callback,
    closure::Closure,
    interpreter::Context,
    object::Object,
    string::String,
    user_data::UserData,
    value::{Function, Number, Value},
};

#[derive(Debug, Clone, Error)]
#[error("type error, expected {expected}, found {found}")]
pub struct TypeError {
    pub expected: Cow<'static, str>,
    pub found: Cow<'static, str>,
}

impl TypeError {
    pub fn new(
        expected: impl Into<Cow<'static, str>>,
        found: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            expected: expected.into(),
            found: found.into(),
        }
    }
}

pub trait IntoValue<'gc> {
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc>;
}

macro_rules! impl_into {
    ($($i:ty),* $(,)?) => {
        $(
            impl<'gc> IntoValue<'gc> for $i {
                fn into_value(self, _: Context<'gc>) -> Value<'gc> {
                    self.into()
                }
            }
        )*
    };
}
impl_into!(
    bool,
    i64,
    f64,
    Number,
    String<'gc>,
    Object<'gc>,
    Array<'gc>,
    Closure<'gc>,
    Callback<'gc>,
    Function<'gc>,
    UserData<'gc>,
    Value<'gc>,
);

macro_rules! impl_int_into {
    ($($i:ty),* $(,)?) => {
        $(
            impl<'gc> IntoValue<'gc> for $i {
                fn into_value(self, _: Context<'gc>) -> Value<'gc> {
                    Value::Integer(self.into())
                }
            }
        )*
    };
}
impl_int_into!(i8, u8, i16, u16, i32, u32);

impl<'gc> IntoValue<'gc> for f32 {
    fn into_value(self, _: Context<'gc>) -> Value<'gc> {
        Value::Float(self.into())
    }
}

impl<'gc> IntoValue<'gc> for isize {
    fn into_value(self, _ctx: Context<'gc>) -> Value<'gc> {
        const { assert!(isize::BITS <= 64) }
        i64::try_from(self).unwrap().into()
    }
}

impl<'gc> IntoValue<'gc> for &'static str {
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        Value::String(ctx.intern(self))
    }
}

impl<'gc> IntoValue<'gc> for StdString {
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        Value::String(ctx.intern(&self))
    }
}

impl<'gc> IntoValue<'gc> for &StdString {
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        Value::String(ctx.intern(self))
    }
}

impl<'gc, T: IntoValue<'gc>> IntoValue<'gc> for Option<T> {
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        match self {
            Some(t) => t.into_value(ctx),
            None => Value::Undefined,
        }
    }
}

impl<'a, 'gc, T> IntoValue<'gc> for &'a Option<T>
where
    &'a T: IntoValue<'gc>,
{
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        match self {
            Some(t) => t.into_value(ctx),
            None => Value::Undefined,
        }
    }
}

impl<'gc, T: IntoValue<'gc>> IntoValue<'gc> for Vec<T> {
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        let array = Array::new(&ctx);
        for (i, v) in self.into_iter().enumerate().rev() {
            array.set(&ctx, i, v.into_value(ctx));
        }
        array.into()
    }
}

impl<'gc, 'a, T> IntoValue<'gc> for &'a [T]
where
    &'a T: IntoValue<'gc>,
{
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        let array = Array::new(&ctx);
        for (i, v) in self.iter().enumerate().rev() {
            array.set(&ctx, i, v.into_value(ctx));
        }
        array.into()
    }
}

impl<'gc, T, const N: usize> IntoValue<'gc> for [T; N]
where
    T: IntoValue<'gc>,
{
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        let array = Array::new(&ctx);
        for (i, v) in self.into_iter().enumerate().rev() {
            array.set(&ctx, i, v.into_value(ctx));
        }
        array.into()
    }
}

pub trait FromValue<'gc>: Sized {
    fn from_value(ctx: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError>;
}

impl<'gc> FromValue<'gc> for Value<'gc> {
    fn from_value(_: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        Ok(value)
    }
}

impl<'gc> FromValue<'gc> for bool {
    fn from_value(_ctx: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        Ok(value.cast_bool())
    }
}

macro_rules! impl_int_from {
    ($($i:ty),* $(,)?) => {
        $(
            impl<'gc> FromValue<'gc> for $i {
                #[allow(irrefutable_let_patterns)]
                fn from_value(
                    _: Context<'gc>,
                    value: Value<'gc>,
                ) -> Result<Self, TypeError> {
                    if let Some(i) = value.cast_integer() {
                        if let Ok(i) = <$i>::try_from(i) {
                            Ok(i)
                        } else {
                            Err(TypeError::new (
                                stringify!($i),
                                "integer out of range",
                            ))
                        }
                    } else {
                        Err(TypeError::new (
                            stringify!($i),
                            value.type_name(),
                        ))
                    }
                }
            }
        )*
    };
}
impl_int_from!(isize, usize, i64, u64, i32, u32, i16, u16, i8, u8);

macro_rules! impl_float_from {
    ($($f:ty),* $(,)?) => {
        $(
            impl<'gc> FromValue<'gc> for $f {
                fn from_value(
                    _: Context<'gc>,
                    value: Value<'gc>,
                ) -> Result<Self, TypeError> {
                    if let Some(n) = value.cast_float() {
                        Ok(n as $f)
                    } else {
                        Err(TypeError::new (
                            stringify!($f),
                            value.type_name(),
                        ))
                    }
                }
            }
        )*
    };
}
impl_float_from!(f32, f64);

impl<'gc> FromValue<'gc> for Number {
    fn from_value(_ctx: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        if let Some(n) = value.to_number() {
            Ok(n)
        } else {
            Err(TypeError::new("numeric value", value.type_name()))
        }
    }
}

macro_rules! impl_from {
    ($([$e:ident $t:ty]),* $(,)?) => {
        $(
            impl<'gc> FromValue<'gc> for $t {
                fn from_value(
                    _: Context<'gc>,
                    value: Value<'gc>,
                ) -> Result<Self, TypeError> {
                    match value {
                        Value::$e(a) => Ok(a),
                        _ => {
                            Err(TypeError::new (
                                stringify!($e),
                                value.type_name(),
                            ))
                        }
                    }
                }
            }
        )*
    };
}
impl_from! {
    [Object Object<'gc>],
    [Array Array<'gc>],
    [Closure Closure<'gc>],
    [Callback Callback<'gc>],
    [UserData UserData<'gc>],
}

impl<'gc> FromValue<'gc> for String<'gc> {
    fn from_value(_ctx: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        if let Some(s) = value.as_string() {
            Ok(s)
        } else {
            Err(TypeError::new("string", value.type_name()))
        }
    }
}

impl<'gc> FromValue<'gc> for Function<'gc> {
    fn from_value(_: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        match value {
            Value::Closure(closure) => Ok(Function::Closure(closure)),
            Value::Callback(callback) => Ok(Function::Callback(callback)),
            v => Err(TypeError::new("callback or closure", v.type_name())),
        }
    }
}

impl<'gc, T: FromValue<'gc>> FromValue<'gc> for Option<T> {
    fn from_value(ctx: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        Ok(if value.is_undefined() {
            None
        } else {
            Some(T::from_value(ctx, value)?)
        })
    }
}

impl<'gc, T: FromValue<'gc>> FromValue<'gc> for Vec<T> {
    fn from_value(ctx: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        if let Value::Array(array) = value {
            (0..array.len())
                .map(|i| T::from_value(ctx, array.get(i).unwrap()))
                .collect()
        } else {
            Err(TypeError::new("array", value.type_name()))
        }
    }
}

impl<'gc, T: FromValue<'gc>, const N: usize> FromValue<'gc> for [T; N] {
    fn from_value(ctx: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        if let Value::Array(array) = value {
            if array.len() != N {
                return Err(TypeError::new(
                    format!("array of length {N}"),
                    format!("array of length {}", array.len()),
                ));
            }

            let mut res: [Option<T>; N] = array::from_fn(|_| None);
            for i in 0..N {
                res[i] = Some(T::from_value(ctx, array.get(i).unwrap())?);
            }
            Ok(res.map(|r| r.unwrap()))
        } else {
            Err(TypeError::new("sequence", value.type_name()))
        }
    }
}

impl<'gc> FromValue<'gc> for StdString {
    fn from_value(ctx: Context<'gc>, value: Value<'gc>) -> Result<Self, TypeError> {
        let str = String::from_value(ctx, value)?;
        Ok(str.as_str().to_owned())
    }
}

pub trait IntoMultiValue<'gc> {
    fn into_multi_value(self, ctx: Context<'gc>) -> impl Iterator<Item = Value<'gc>>;
}

impl<'gc, T: IntoValue<'gc>> IntoMultiValue<'gc> for T {
    fn into_multi_value(self, ctx: Context<'gc>) -> impl Iterator<Item = Value<'gc>> {
        iter::once(self.into_value(ctx))
    }
}

pub trait FromMultiValue<'gc>: Sized {
    fn from_multi_value(
        ctx: Context<'gc>,
        values: impl Iterator<Item = Value<'gc>>,
    ) -> Result<Self, TypeError>;
}

impl<'gc, T: FromValue<'gc>> FromMultiValue<'gc> for T {
    fn from_multi_value(
        ctx: Context<'gc>,
        mut values: impl Iterator<Item = Value<'gc>>,
    ) -> Result<Self, TypeError> {
        T::from_value(ctx, values.next().unwrap_or(Value::Undefined))
    }
}

impl<'gc, T: IntoMultiValue<'gc>, E: IntoValue<'gc>> IntoMultiValue<'gc> for Result<T, E> {
    fn into_multi_value(self, ctx: Context<'gc>) -> impl Iterator<Item = Value<'gc>> {
        enum ResultIter<'gc, I> {
            Ok(I),
            Err(iter::Once<Value<'gc>>),
        }

        impl<'gc, I> Iterator for ResultIter<'gc, I>
        where
            I: Iterator<Item = Value<'gc>>,
        {
            type Item = Value<'gc>;

            fn next(&mut self) -> Option<Self::Item> {
                match self {
                    ResultIter::Ok(i) => i.next(),
                    ResultIter::Err(i) => i.next(),
                }
            }
        }

        match self {
            Ok(v) => iter::once(true.into()).chain(ResultIter::Ok(v.into_multi_value(ctx))),
            Err(e) => {
                iter::once(false.into()).chain(ResultIter::Err(iter::once(e.into_value(ctx))))
            }
        }
    }
}

/// A marker newtype that converts to / from *multiple* values.
///
/// A `Vec<T>` has [`IntoValue`] / [`FromValue`] implementations that conver to / from an [`Array`],
/// while a `Variadic<Vec<T>>` has [`IntoMultiValue`] and [`FromMultiValue`] implementations that
/// convert to / from multiple values at once.
///
/// Use this to provide a variable number of arguments to a function, or to collect multiple return
/// values into a single container.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Variadic<T>(pub T);

impl<T> ops::Deref for Variadic<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> ops::DerefMut for Variadic<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: IntoIterator> IntoIterator for Variadic<T> {
    type Item = T::Item;
    type IntoIter = T::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Variadic<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        (&self.0).into_iter()
    }
}

impl<I, T: FromIterator<I>> FromIterator<I> for Variadic<T> {
    fn from_iter<It: IntoIterator<Item = I>>(iter: It) -> Self {
        Self(T::from_iter(iter))
    }
}

impl<'gc, T: IntoIterator> IntoMultiValue<'gc> for Variadic<T>
where
    T::Item: IntoValue<'gc>,
{
    fn into_multi_value(self, ctx: Context<'gc>) -> impl Iterator<Item = Value<'gc>> {
        self.0.into_iter().map(move |v| v.into_value(ctx))
    }
}

impl<'a, 'gc, T> IntoMultiValue<'gc> for &'a Variadic<T>
where
    &'a T: IntoIterator,
    <&'a T as IntoIterator>::Item: IntoValue<'gc>,
{
    fn into_multi_value(self, ctx: Context<'gc>) -> impl Iterator<Item = Value<'gc>> {
        self.0.into_iter().map(move |v| v.into_value(ctx))
    }
}

impl<'gc, I: FromValue<'gc>> FromMultiValue<'gc> for Variadic<Vec<I>> {
    fn from_multi_value(
        ctx: Context<'gc>,
        values: impl Iterator<Item = Value<'gc>>,
    ) -> Result<Self, TypeError> {
        values.map(|v| I::from_value(ctx, v)).collect()
    }
}

impl<'gc, I: FromValue<'gc>, const N: usize> FromMultiValue<'gc> for Variadic<[I; N]> {
    fn from_multi_value(
        ctx: Context<'gc>,
        mut values: impl Iterator<Item = Value<'gc>>,
    ) -> Result<Self, TypeError> {
        let mut res: [Option<I>; N] = array::from_fn(|_| None);
        for i in 0..N {
            res[i] = Some(I::from_value(
                ctx,
                values.next().unwrap_or(Value::Undefined),
            )?);
        }

        Ok(Self(res.map(|v| v.unwrap())))
    }
}

macro_rules! impl_tuple {
    ($($name:ident),* $(,)?) => (
        impl<'gc, $($name,)*> IntoMultiValue<'gc> for ($($name,)*)
        where
            $($name: IntoMultiValue<'gc>,)*
        {
            #[allow(unused_variables)]
            #[allow(unused_mut)]
            #[allow(non_snake_case)]
            fn into_multi_value(self, ctx: Context<'gc>) -> impl Iterator<Item = Value<'gc>> {
                let ($($name,)*) = self;
                let i = iter::empty();
                $(
                    let i = i.chain($name.into_multi_value(ctx));
                )*
                i
            }
        }

        impl<'gc, $($name,)*> FromMultiValue<'gc> for ($($name,)*)
            where $($name: FromMultiValue<'gc>,)*
        {
            #[allow(unused_variables)]
            #[allow(unused_mut)]
            #[allow(non_snake_case)]
            fn from_multi_value(
                ctx: Context<'gc>,
                mut values: impl Iterator<Item = Value<'gc>>,
            ) -> Result<Self, TypeError> {
                $(let $name = FromMultiValue::from_multi_value(ctx, &mut values)?;)*
                Ok(($($name,)*))
            }
        }
    );
}

macro_rules! smaller_tuples_too {
    ($m: ident, $ty: ident) => {
        $m!{}
        $m!{$ty}
    };

    ($m: ident, $ty: ident, $($tt: ident),*) => {
        smaller_tuples_too!{$m, $($tt),*}
        $m!{$ty, $($tt),*}
    };
}

smaller_tuples_too!(impl_tuple, P, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);
