use std::hash::{Hash, Hasher};

use fabricator_vm::value::Number;
use gc_arena::Collect;

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub enum Constant<S> {
    Undefined,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(S),
}

impl<S: PartialEq> PartialEq for Constant<S> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Constant::Undefined, Constant::Undefined) => true,
            (Constant::Boolean(a), Constant::Boolean(b)) => a == b,
            (Constant::Integer(a), Constant::Integer(b)) => a == b,
            (Constant::Float(a), Constant::Float(b)) => a.to_bits() == b.to_bits(),
            (Constant::String(a), Constant::String(b)) => a == b,
            _ => false,
        }
    }
}

impl<S: Eq> Eq for Constant<S> {}

impl<S: Hash> Hash for Constant<S> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Constant::Undefined => {
                0u8.hash(state);
            }
            Constant::Boolean(b) => {
                1u8.hash(state);
                b.hash(state);
            }
            Constant::Integer(i) => {
                2u8.hash(state);
                i.hash(state);
            }
            Constant::Float(f) => {
                3u8.hash(state);
                f.to_bits().hash(state);
            }
            Constant::String(s) => {
                4u8.hash(state);
                s.hash(state);
            }
        }
    }
}

impl<S> From<bool> for Constant<S> {
    fn from(b: bool) -> Self {
        Constant::Boolean(b)
    }
}

impl<S> From<i64> for Constant<S> {
    fn from(i: i64) -> Self {
        Constant::Integer(i)
    }
}

impl<S> From<f64> for Constant<S> {
    fn from(f: f64) -> Self {
        Constant::Float(f)
    }
}

impl<S> From<Number> for Constant<S> {
    fn from(n: Number) -> Self {
        match n {
            Number::Integer(i) => Self::Integer(i),
            Number::Float(f) => Self::Float(f),
        }
    }
}

impl<S> Constant<S> {
    #[inline]
    #[must_use]
    pub fn is_undefined(&self) -> bool {
        matches!(&self, Constant::Undefined)
    }

    #[inline]
    #[must_use]
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Constant::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Constant::Integer(i) => Some(*i),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Constant::Float(f) => Some(*f),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub fn as_string(&self) -> Option<&S> {
        match self {
            Constant::String(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub fn to_number(&self) -> Option<Number> {
        match self {
            Constant::Boolean(b) => Some(Number::Integer(if *b { 1 } else { 0 })),
            Constant::Integer(i) => Some(Number::Integer(*i)),
            Constant::Float(f) => Some(Number::Float(*f)),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub fn cast_bool(&self) -> bool {
        match *self {
            Constant::Undefined => false,
            Constant::Boolean(b) => b,
            Constant::Integer(i) => i > 0,
            Constant::Float(f) => f > 0.5,
            _ => true,
        }
    }

    #[inline]
    pub fn negate(&self) -> Option<Constant<S>> {
        Some(self.to_number()?.negate().into())
    }

    #[inline]
    pub fn add(&self, other: &Constant<S>) -> Option<Constant<S>> {
        Some(self.to_number()?.add(other.to_number()?).into())
    }

    #[inline]
    pub fn sub(&self, other: &Constant<S>) -> Option<Constant<S>> {
        Some(self.to_number()?.sub(other.to_number()?).into())
    }

    #[inline]
    pub fn mult(&self, other: &Constant<S>) -> Option<Constant<S>> {
        Some(self.to_number()?.mult(other.to_number()?).into())
    }

    #[inline]
    pub fn div(&self, other: &Constant<S>) -> Option<Constant<S>> {
        Some(self.to_number()?.div(other.to_number()?).into())
    }

    #[inline]
    pub fn idiv(&self, other: &Constant<S>) -> Option<i64> {
        Some(self.to_number()?.idiv(other.to_number()?).into())
    }

    #[inline]
    pub fn rem(&self, other: &Constant<S>) -> Option<Constant<S>> {
        Some(self.to_number()?.rem(other.to_number()?).into())
    }

    #[inline]
    pub fn equal(&self, other: &Constant<S>) -> bool
    where
        S: Eq,
    {
        match (self, other) {
            (Constant::Undefined, Constant::Undefined) => true,
            (Constant::String(a), Constant::String(b)) => a == b,
            _ => {
                if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
                    a == b
                } else {
                    false
                }
            }
        }
    }

    #[inline]
    pub fn less_than(&self, other: &Constant<S>) -> Option<bool> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Some(a < b)
        } else {
            None
        }
    }

    #[inline]
    pub fn less_equal(&self, other: &Constant<S>) -> Option<bool> {
        if let (Some(a), Some(b)) = (self.to_number(), other.to_number()) {
            Some(a <= b)
        } else {
            None
        }
    }

    #[inline]
    pub fn and(&self, other: &Constant<S>) -> bool {
        self.cast_bool() && other.cast_bool()
    }

    #[inline]
    pub fn or(&self, other: &Constant<S>) -> bool {
        self.cast_bool() || other.cast_bool()
    }

    #[inline]
    pub fn xor(&self, other: &Constant<S>) -> bool {
        self.cast_bool() ^ other.cast_bool()
    }

    #[inline]
    pub fn bit_negate(&self) -> Option<i64> {
        Some(self.to_number()?.bit_negate())
    }

    #[inline]
    pub fn bit_and(&self, other: &Constant<S>) -> Option<i64> {
        Some(self.to_number()?.bit_and(other.to_number()?))
    }

    #[inline]
    pub fn bit_or(&self, other: &Constant<S>) -> Option<i64> {
        Some(self.to_number()?.bit_or(other.to_number()?))
    }

    #[inline]
    pub fn bit_xor(&self, other: &Constant<S>) -> Option<i64> {
        Some(self.to_number()?.bit_xor(other.to_number()?))
    }

    #[inline]
    pub fn bit_shift_left(&self, other: &Constant<S>) -> Option<i64> {
        Some(self.to_number()?.bit_shift_left(other.to_number()?))
    }

    #[inline]
    pub fn bit_shift_right(&self, other: &Constant<S>) -> Option<i64> {
        Some(self.to_number()?.bit_shift_right(other.to_number()?))
    }

    #[inline]
    pub fn null_coalesce<'a>(&'a self, other: &'a Constant<S>) -> &'a Constant<S> {
        if self.is_undefined() { other } else { self }
    }

    #[must_use]
    pub fn as_string_ref(&self) -> Constant<&S> {
        match self {
            Constant::Undefined => Constant::Undefined,
            Constant::Boolean(b) => Constant::Boolean(*b),
            Constant::Integer(i) => Constant::Integer(*i),
            Constant::Float(f) => Constant::Float(*f),
            Constant::String(s) => Constant::String(s),
        }
    }

    #[must_use]
    pub fn map_string<S2>(self, map: impl Fn(S) -> S2) -> Constant<S2> {
        match self {
            Constant::Undefined => Constant::Undefined,
            Constant::Boolean(b) => Constant::Boolean(b),
            Constant::Integer(i) => Constant::Integer(i),
            Constant::Float(f) => Constant::Float(f),
            Constant::String(s) => Constant::String(map(s)),
        }
    }
}

impl<S: AsRef<str>> Constant<S> {
    pub fn as_str(&self) -> Constant<&str> {
        match self {
            Constant::Undefined => Constant::Undefined,
            Constant::Boolean(b) => Constant::Boolean(*b),
            Constant::Integer(i) => Constant::Integer(*i),
            Constant::Float(f) => Constant::Float(*f),
            Constant::String(s) => Constant::String(s.as_ref()),
        }
    }
}
