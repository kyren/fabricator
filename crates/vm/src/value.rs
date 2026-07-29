use std::{cmp, fmt};

use gc_arena::{Collect, Mutation};
use thiserror::Error;

use crate::{
    array::Array, callback::Callback, closure::Closure, interpreter::Context, object::Object,
    string::String, user_data::UserData,
};

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub enum Function<'gc> {
    Closure(Closure<'gc>),
    Callback(Callback<'gc>),
}

impl<'gc> Function<'gc> {
    pub fn this(self) -> Option<Value<'gc>> {
        match self {
            Function::Closure(closure) => closure.this(),
            Function::Callback(callback) => callback.this(),
        }
    }

    pub fn rebind(self, mc: &Mutation<'gc>, this: Option<Value<'gc>>) -> Self {
        match self {
            Function::Closure(closure) => Function::Closure(closure.rebind(mc, this)),
            Function::Callback(callback) => Function::Callback(callback.rebind(mc, this)),
        }
    }
}

impl<'gc> From<Closure<'gc>> for Function<'gc> {
    fn from(closure: Closure<'gc>) -> Self {
        Self::Closure(closure)
    }
}

impl<'gc> From<Callback<'gc>> for Function<'gc> {
    fn from(callback: Callback<'gc>) -> Self {
        Self::Callback(callback)
    }
}

/// The representation for all values in FML.
///
/// There are three levels of conversion methods provided on `Value` to convert them to concrete
/// types.
///   1) `as_xxx` conversions are perfect conversions that return exact values, there is one per
///       variant type (including `is_undefined`).
///   2) `cast_xxx` conversions are generally reasonable conversions to do implicitly. They include
///      things that match normal GML semantics like evaluating value truthiness, casting bools to
///      numbers, and casting between numeric types.
///   3) `coerce_xxx` conversions are expensive and should not usually be done implicitly except in
///      specific circumstances. They include things like coercing scalar values into strings and
///      userdata type coercions.
#[derive(Copy, Clone, Default, Collect)]
#[collect(no_drop)]
pub enum Value<'gc> {
    #[default]
    Undefined,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String<'gc>),
    Object(Object<'gc>),
    Array(Array<'gc>),
    Closure(Closure<'gc>),
    Callback(Callback<'gc>),
    UserData(UserData<'gc>),
}

impl<'gc> PartialEq for Value<'gc> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.equal(*other)
    }
}

impl<'gc> fmt::Debug for Value<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Undefined => write!(f, "`undefined`"),
            Value::Boolean(b) => write!(f, "`{b}`"),
            Value::Integer(i) => write!(f, "`{i}`"),
            Value::Float(n) => write!(f, "`{n}`"),
            Value::String(s) => write!(f, "`{:?}`", s),
            Value::Object(object) => write!(f, "<object {:p}>", object.into_inner()),
            Value::Array(array) => write!(f, "<array {:p}>", array.into_inner()),
            Value::Closure(closure) => write!(f, "<closure {:p}>", closure.into_inner()),
            Value::Callback(callback) => write!(f, "<callback {:p}>", callback.into_inner()),
            Value::UserData(user_data) => write!(f, "<user_data {:p}>", user_data.into_inner()),
        }
    }
}

impl<'gc> fmt::Display for Value<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Undefined => write!(f, "undefined"),
            Value::Boolean(b) => write!(f, "{b}"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Float(n) => write!(f, "{n}"),
            Value::String(s) => write!(f, "{:?}", s),
            Value::Object(object) => write!(f, "<object {:p}>", object.into_inner()),
            Value::Array(array) => write!(f, "<array {:p}>", array.into_inner()),
            Value::Closure(closure) => write!(f, "<closure {:p}>", closure.into_inner()),
            Value::Callback(callback) => write!(f, "<callback {:p}>", callback.into_inner()),
            Value::UserData(user_data) => write!(f, "<user_data {:p}>", user_data.into_inner()),
        }
    }
}

impl<'gc> From<bool> for Value<'gc> {
    fn from(b: bool) -> Self {
        Value::Boolean(b)
    }
}

impl<'gc> From<i64> for Value<'gc> {
    fn from(i: i64) -> Self {
        Value::Integer(i)
    }
}

impl<'gc> From<f64> for Value<'gc> {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}

impl<'gc> From<Number> for Value<'gc> {
    fn from(n: Number) -> Self {
        match n {
            Number::Integer(i) => Value::Integer(i),
            Number::Float(f) => Value::Float(f),
        }
    }
}

impl<'gc> From<String<'gc>> for Value<'gc> {
    fn from(s: String<'gc>) -> Self {
        Value::String(s)
    }
}

impl<'gc> From<Object<'gc>> for Value<'gc> {
    fn from(o: Object<'gc>) -> Self {
        Value::Object(o)
    }
}

impl<'gc> From<Array<'gc>> for Value<'gc> {
    fn from(a: Array<'gc>) -> Self {
        Value::Array(a)
    }
}

impl<'gc> From<Closure<'gc>> for Value<'gc> {
    fn from(c: Closure<'gc>) -> Self {
        Value::Closure(c)
    }
}

impl<'gc> From<Callback<'gc>> for Value<'gc> {
    fn from(c: Callback<'gc>) -> Self {
        Value::Callback(c)
    }
}

impl<'gc> From<UserData<'gc>> for Value<'gc> {
    fn from(u: UserData<'gc>) -> Self {
        Value::UserData(u)
    }
}

impl<'gc> From<Function<'gc>> for Value<'gc> {
    fn from(func: Function<'gc>) -> Self {
        match func {
            Function::Closure(closure) => Value::Closure(closure),
            Function::Callback(callback) => Value::Callback(callback),
        }
    }
}

#[derive(Debug, Error)]
#[error("`Value` operation was performed on the wrong type")]
pub struct BadValueType;

impl<'gc> Value<'gc> {
    #[must_use]
    #[inline]
    pub fn type_name(self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Boolean(_) => "boolean",
            Value::Integer(_) => "integer",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Object(_) => "object",
            Value::Array(_) => "array",
            Value::Closure(_) => "closure",
            Value::Callback(_) => "callback",
            Value::UserData(_) => "userdata",
        }
    }

    /// Returns `true` if `self` is `Value::Undefined`.
    #[must_use]
    #[inline]
    pub fn is_undefined(self) -> bool {
        matches!(self, Value::Undefined)
    }

    /// The inverse of [`Value::is_undefined`], returns `true` if `self` is anything other than
    /// `Value::Undefined`.
    #[must_use]
    #[inline]
    pub fn is_defined(self) -> bool {
        !matches!(self, Value::Undefined)
    }

    #[must_use]
    #[inline]
    pub fn as_boolean(self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(b),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_integer(self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(i),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_float(self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(f),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_number(self) -> Option<Number> {
        match self {
            Value::Integer(i) => Some(Number::Integer(i)),
            Value::Float(f) => Some(Number::Float(f)),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_string(self) -> Option<String<'gc>> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_object(self) -> Option<Object<'gc>> {
        match self {
            Value::Object(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_array(self) -> Option<Array<'gc>> {
        match self {
            Value::Array(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_closure(self) -> Option<Closure<'gc>> {
        match self {
            Value::Closure(c) => Some(c),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_callback(self) -> Option<Callback<'gc>> {
        match self {
            Value::Callback(c) => Some(c),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_function(self) -> Option<Function<'gc>> {
        match self {
            Value::Closure(closure) => Some(Function::Closure(closure)),
            Value::Callback(callback) => Some(Function::Callback(callback)),
            _ => None,
        }
    }

    #[must_use]
    #[inline]
    pub fn as_userdata(self) -> Option<UserData<'gc>> {
        match self {
            Value::UserData(userdata) => Some(userdata),
            _ => None,
        }
    }

    /// Return numeric values as integers or floats.
    ///
    /// Integers and floats are returned as themselves, booleans return integral 0 or 1.
    #[must_use]
    #[inline]
    pub fn to_number(self) -> Option<Number> {
        match self {
            Value::Boolean(b) => Some(Number::Integer(if b { 1 } else { 0 })),
            Value::Integer(i) => Some(Number::Integer(i)),
            Value::Float(f) => Some(Number::Float(f)),
            _ => None,
        }
    }

    /// Interpret any value as a boolean.
    ///
    /// Boolean values are returned as themselves, `Value::Undefined` returns false, integers and
    /// floats return true if they are greater than 0.5 and false otherwise, and all other value
    /// types return true.
    #[must_use]
    #[inline]
    pub fn cast_bool(self) -> bool {
        match self {
            Value::Undefined => false,
            Value::Boolean(b) => b,
            Value::Integer(i) => i > 0,
            Value::Float(f) => f > 0.5,
            _ => true,
        }
    }

    /// Interpret numeric values as integers.
    ///
    /// Integers are returned as themselves, booleans return 0 or 1, and floats return their integer
    /// part.
    #[must_use]
    #[inline]
    pub fn cast_integer(self) -> Option<i64> {
        self.to_number().map(Number::cast_integer)
    }

    /// Interpret numeric values as floats.
    ///
    /// Floats are returned as themselves, booleans return 0.0 or 1.0, and integers are converted
    /// to floats.
    #[must_use]
    #[inline]
    pub fn cast_float(self) -> Option<f64> {
        self.to_number().map(Number::cast_float)
    }

    /// Coerce values into strings.
    ///
    /// Strings are returned as themselves, booleans return either `"true"` or `"false"`, numeric
    /// values are printed, and userdata values are coerced if they implement string coercion.
    #[must_use]
    #[inline]
    pub fn coerce_string(self, ctx: Context<'gc>) -> Option<String<'gc>> {
        match self {
            Value::Boolean(b) => Some(ctx.intern(if b { "true" } else { "false" })),
            Value::Integer(i) => Some(ctx.intern(&i.to_string())),
            Value::Float(f) => Some(ctx.intern(&f.to_string())),
            Value::String(s) => Some(s),
            Value::UserData(u) => u.coerce_string(ctx),
            _ => None,
        }
    }

    /// Coerce values into integers.
    ///
    /// This is similar to [`Value::cast_integer`] with the addition of parsing strings as integers
    /// if possible, and coercing userdata to integers if they implement integer coersion.
    #[must_use]
    #[inline]
    pub fn coerce_integer(self, ctx: Context<'gc>) -> Option<i64> {
        if let Some(i) = self.cast_integer() {
            Some(i)
        } else {
            match self {
                Value::String(s) => s.parse().ok(),
                Value::UserData(u) => u.coerce_integer(ctx),
                _ => None,
            }
        }
    }

    /// Coerce values into floats.
    ///
    /// This is similar to [`Value::cast_float`] with the addition of parsing strings as floats if
    /// possible, and coercing userdata to floats if they implement float coersion.
    #[must_use]
    #[inline]
    pub fn coerce_float(self, ctx: Context<'gc>) -> Option<f64> {
        if let Some(i) = self.cast_float() {
            Some(i)
        } else {
            match self {
                Value::String(s) => s.parse().ok(),
                Value::UserData(u) => u.coerce_float(ctx),
                _ => None,
            }
        }
    }

    /// If both values are numeric, return the two values added together. If both values are
    /// strings, appends them.
    #[must_use]
    #[inline]
    pub fn add_or_append(
        self,
        ctx: Context<'gc>,
        other: Value<'gc>,
    ) -> Result<Value<'gc>, BadValueType> {
        if let Ok(r) = self.add(other) {
            Ok(r)
        } else if let (Value::String(a), Value::String(b)) = (self, other) {
            Ok(ctx.intern(&format!("{a}{b}")).into())
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn negate(self) -> Result<Value<'gc>, BadValueType> {
        Ok(self.to_number().ok_or(BadValueType)?.negate().into())
    }

    #[must_use]
    #[inline]
    pub fn add(self, other: Value<'gc>) -> Result<Value<'gc>, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.add(b).into())
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn sub(self, other: Value<'gc>) -> Result<Value<'gc>, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.sub(b).into())
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn mult(self, other: Value<'gc>) -> Result<Value<'gc>, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.mult(b).into())
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn div(self, other: Value<'gc>) -> Result<Value<'gc>, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.div(b).into())
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn idiv(self, other: Value<'gc>) -> Result<Result<i64, DivByZero>, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.idiv(b))
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn rem(self, other: Value<'gc>) -> Result<Result<Value<'gc>, DivByZero>, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.rem(b).map(|n| n.into()))
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn equal(self, other: Value<'gc>) -> bool {
        match (self, other) {
            (Value::Undefined, Value::Undefined) => true,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Integer(a), Value::Integer(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Closure(a), Value::Closure(b)) => a == b,
            (Value::Callback(a), Value::Callback(b)) => a == b,
            (Value::UserData(a), Value::UserData(b)) => a == b,
            _ => {
                if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
                    a == b
                } else {
                    false
                }
            }
        }
    }

    #[must_use]
    #[inline]
    pub fn less_than(self, other: Value<'gc>) -> Result<bool, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a < b)
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn less_equal(self, other: Value<'gc>) -> Result<bool, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a <= b)
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn and(&self, other: Value<'gc>) -> bool {
        self.cast_bool() && other.cast_bool()
    }

    #[must_use]
    #[inline]
    pub fn or(&self, other: Value<'gc>) -> bool {
        self.cast_bool() || other.cast_bool()
    }

    #[must_use]
    #[inline]
    pub fn xor(&self, other: Value<'gc>) -> bool {
        self.cast_bool() ^ other.cast_bool()
    }

    #[inline]
    pub fn bit_negate(&self) -> Result<i64, BadValueType> {
        Ok(self.to_number().ok_or(BadValueType)?.bit_negate())
    }

    #[must_use]
    #[inline]
    pub fn bit_and(&self, other: Value<'gc>) -> Result<i64, BadValueType> {
        Ok(self
            .to_number()
            .ok_or(BadValueType)?
            .bit_and(other.to_number().ok_or(BadValueType)?))
    }

    #[must_use]
    #[inline]
    pub fn bit_or(&self, other: Value<'gc>) -> Result<i64, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.bit_or(b))
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn bit_xor(&self, other: Value<'gc>) -> Result<i64, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.bit_xor(b))
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn bit_shift_left(&self, other: Value<'gc>) -> Result<i64, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.bit_shift_left(b))
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn bit_shift_right(&self, other: Value<'gc>) -> Result<i64, BadValueType> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Ok(a.bit_shift_right(b))
        } else {
            Err(BadValueType)
        }
    }

    #[must_use]
    #[inline]
    pub fn null_coalesce(self, other: Value<'gc>) -> Value<'gc> {
        if self.is_undefined() { other } else { self }
    }
}

#[derive(Debug, Error)]
#[error("integral divide by zero")]
pub struct DivByZero;

/// A numeric value that has an exact representation in [`Value`].
#[derive(Debug, Copy, Clone)]
pub enum Number {
    Integer(i64),
    Float(f64),
}

impl PartialEq for Number {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other).is_some_and(|o| o.is_eq())
    }
}

impl PartialOrd for Number {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        fn cmp_int_float(a: i64, b: f64) -> Option<cmp::Ordering> {
            // Maximum integer value such that the value may be losslessly converted into a float,
            // and additionally no other integer value will convert to that same float.
            const MAX_EXACT_INTEGER: u64 = (1 << f64::MANTISSA_DIGITS) - 1;

            if a.unsigned_abs() <= MAX_EXACT_INTEGER {
                (a as f64).partial_cmp(&b)
            } else if b.is_finite() {
                // If `b`'s magnitude is less than `MAX_EXACT_INTEGER`, then since `a` is >
                // `MAX_EXACT_INTEGER`, casting to an i64 will not change the result.
                //
                // If `b`'s magnitude is greater than or equal to `MAX_EXACT_INTEGER` then it will
                // be an exact integer anyway.
                debug_assert!(b.abs() < MAX_EXACT_INTEGER as f64 || b.fract() == 0.0);
                Some(a.cmp(&(b as i64)))
            } else {
                // `b` is either infinite or NaN, so just compare with zero.
                0.0.partial_cmp(&b)
            }
        }

        match (*self, *other) {
            (Number::Integer(a), Number::Integer(b)) => a.partial_cmp(&b),
            (Number::Float(a), Number::Float(b)) => a.partial_cmp(&b),
            (Number::Integer(a), Number::Float(b)) => cmp_int_float(a, b),
            (Number::Float(a), Number::Integer(b)) => cmp_int_float(b, a).map(|o| o.reverse()),
        }
    }
}

impl Number {
    #[inline]
    #[must_use]
    pub fn as_integer(self) -> Option<i64> {
        if let Number::Integer(v) = self {
            Some(v)
        } else {
            None
        }
    }

    #[inline]
    #[must_use]
    pub fn as_float(self) -> Option<f64> {
        if let Number::Float(v) = self {
            Some(v)
        } else {
            None
        }
    }

    #[inline]
    #[must_use]
    pub fn cast_integer(self) -> i64 {
        match self {
            Number::Integer(i) => i,
            Number::Float(f) => f as i64,
        }
    }

    #[inline]
    #[must_use]
    pub fn cast_float(self) -> f64 {
        match self {
            Number::Integer(i) => i as f64,
            Number::Float(f) => f,
        }
    }

    #[inline]
    #[must_use]
    pub fn negate(self) -> Number {
        match self {
            Number::Integer(i) => Number::Integer(i.wrapping_neg()),
            Number::Float(f) => Number::Float(-f),
        }
    }

    #[inline]
    #[must_use]
    pub fn add(self, other: Number) -> Number {
        if let (Some(a), Some(b)) = (self.as_integer(), other.as_integer()) {
            Number::Integer(a.wrapping_add(b))
        } else {
            Number::Float(self.cast_float() + other.cast_float())
        }
    }

    #[inline]
    #[must_use]
    pub fn sub(self, other: Number) -> Number {
        if let (Some(a), Some(b)) = (self.as_integer(), other.as_integer()) {
            Number::Integer(a.wrapping_sub(b))
        } else {
            Number::Float(self.cast_float() - other.cast_float())
        }
    }

    #[inline]
    #[must_use]
    pub fn mult(self, other: Number) -> Number {
        if let (Some(a), Some(b)) = (self.as_integer(), other.as_integer()) {
            Number::Integer(a.wrapping_mul(b))
        } else {
            Number::Float(self.cast_float() * other.cast_float())
        }
    }

    #[inline]
    #[must_use]
    pub fn div(self, other: Number) -> f64 {
        self.cast_float() / other.cast_float()
    }

    #[inline]
    #[must_use]
    pub fn idiv(self, other: Number) -> Result<i64, DivByZero> {
        let a = self.cast_integer();
        let b = other.cast_integer();
        if b == 0 {
            Err(DivByZero)
        } else {
            Ok(a.wrapping_div(b))
        }
    }

    #[inline]
    #[must_use]
    pub fn rem(self, other: Number) -> Result<Number, DivByZero> {
        if let (Some(a), Some(b)) = (self.as_integer(), other.as_integer()) {
            if b == 0 {
                Err(DivByZero)
            } else {
                Ok(Number::Integer(a.wrapping_rem(b)))
            }
        } else {
            Ok(Number::Float(self.cast_float() % other.cast_float()))
        }
    }

    #[inline]
    #[must_use]
    pub fn bit_negate(&self) -> i64 {
        !self.cast_integer()
    }

    #[inline]
    #[must_use]
    pub fn bit_and(&self, other: Number) -> i64 {
        self.cast_integer() & other.cast_integer()
    }

    #[inline]
    #[must_use]
    pub fn bit_or(&self, other: Number) -> i64 {
        self.cast_integer() | other.cast_integer()
    }

    #[inline]
    #[must_use]
    pub fn bit_xor(&self, other: Number) -> i64 {
        self.cast_integer() ^ other.cast_integer()
    }

    #[inline]
    #[must_use]
    pub fn bit_shift_left(&self, other: Number) -> i64 {
        self.cast_integer()
            .wrapping_shl(other.cast_integer() as u32)
    }

    #[inline]
    #[must_use]
    pub fn bit_shift_right(&self, other: Number) -> i64 {
        self.cast_integer()
            .wrapping_shr(other.cast_integer() as u32)
    }
}
