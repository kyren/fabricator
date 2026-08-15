use std::{
    cell::{Ref, RefMut},
    hash, iter,
    ops::{self, RangeBounds},
    slice, vec,
};

use gc_arena::{Collect, Gc, Mutation, RefLock};
use thiserror::Error;

use crate::{conversion::IntoValue, interpreter::Context, value::Value};

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct Array<'gc>(Gc<'gc, ArrayInner<'gc>>);

pub type ArrayInner<'gc> = RefLock<ArrayVec<'gc>>;

impl<'gc> PartialEq for Array<'gc> {
    fn eq(&self, other: &Self) -> bool {
        Gc::ptr_eq(self.0, other.0)
    }
}

impl<'gc> Eq for Array<'gc> {}

impl<'gc> hash::Hash for Array<'gc> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        Gc::as_ptr(self.0).hash(state)
    }
}

#[derive(Debug, Error)]
#[error("`Array` is already borrowed mutably")]
pub struct ArrayBorrowError;

#[derive(Debug, Error)]
#[error("`Array` is already borrowed")]
pub struct ArrayBorrowMutError;

impl<'gc> Array<'gc> {
    pub fn new(mc: &Mutation<'gc>) -> Self {
        Self::from_vec(mc, ArrayVec::new())
    }

    pub fn from_vec(mc: &Mutation<'gc>, vec: ArrayVec<'gc>) -> Self {
        Self(Gc::new(mc, ArrayInner::new(vec)))
    }

    #[inline]
    pub fn from_iter(mc: &Mutation<'gc>, iter: impl IntoIterator<Item = Value<'gc>>) -> Self {
        Self::from_vec(mc, ArrayVec::from_iter(iter))
    }

    #[inline]
    pub fn from_inner(inner: Gc<'gc, ArrayInner<'gc>>) -> Self {
        Self(inner)
    }

    #[inline]
    pub fn into_inner(self) -> Gc<'gc, ArrayInner<'gc>> {
        self.0
    }

    #[inline]
    pub fn borrow(&self) -> Ref<'_, ArrayVec<'gc>> {
        self.0.borrow()
    }

    #[inline]
    pub fn borrow_mut(&self, mc: &Mutation<'gc>) -> RefMut<'_, ArrayVec<'gc>> {
        self.0.borrow_mut(mc)
    }

    #[inline]
    pub fn try_borrow(&self) -> Result<Ref<'_, ArrayVec<'gc>>, ArrayBorrowError> {
        self.0.try_borrow().map_err(|_| ArrayBorrowError)
    }

    #[inline]
    pub fn try_borrow_mut(
        &self,
        mc: &Mutation<'gc>,
    ) -> Result<RefMut<'_, ArrayVec<'gc>>, ArrayBorrowMutError> {
        self.0.try_borrow_mut(mc).map_err(|_| ArrayBorrowMutError)
    }
}

#[derive(Debug, Default, Collect)]
#[collect(no_drop)]
pub struct ArrayVec<'gc> {
    inner: Vec<Value<'gc>>,
}

impl<'gc> IntoValue<'gc> for ArrayVec<'gc> {
    #[inline]
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        Array::from_vec(&ctx, self).into()
    }
}

impl<'gc> ArrayVec<'gc> {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    pub fn resize(&mut self, new_len: usize, value: impl Into<Value<'gc>>) {
        self.inner.resize(new_len, value.into());
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<Value<'gc>> {
        self.inner.get(index).copied()
    }

    #[inline]
    pub fn set(&mut self, index: usize, value: impl Into<Value<'gc>>) {
        if index >= self.inner.len() {
            self.inner.resize(index + 1, Value::Undefined);
        }
        self.inner[index] = value.into();
    }

    #[inline]
    pub fn push(&mut self, value: impl Into<Value<'gc>>) {
        self.inner.push(value.into());
    }

    #[inline]
    pub fn pop(&mut self) -> Option<Value<'gc>> {
        self.inner.pop()
    }

    #[inline]
    pub fn insert(&mut self, index: usize, value: impl Into<Value<'gc>>) {
        self.inner.insert(index, value.into());
    }

    #[inline]
    pub fn iter(&self) -> <&Self as IntoIterator>::IntoIter {
        self.into_iter()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> <&mut Self as IntoIterator>::IntoIter {
        self.into_iter()
    }

    #[inline]
    pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) -> vec::Drain<'_, Value<'gc>> {
        self.inner.drain(range)
    }
}

impl<'gc> ops::Deref for ArrayVec<'gc> {
    type Target = [Value<'gc>];

    fn deref(&self) -> &Self::Target {
        &self.inner[..]
    }
}

impl<'gc> ops::DerefMut for ArrayVec<'gc> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner[..]
    }
}

impl<'gc, 'a> IntoIterator for &'a ArrayVec<'gc> {
    type Item = Value<'gc>;
    type IntoIter = iter::Copied<slice::Iter<'a, Value<'gc>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter().copied()
    }
}

impl<'gc, 'a> IntoIterator for &'a mut ArrayVec<'gc> {
    type Item = &'a mut Value<'gc>;
    type IntoIter = slice::IterMut<'a, Value<'gc>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter_mut()
    }
}

impl<'gc> FromIterator<Value<'gc>> for ArrayVec<'gc> {
    #[inline]
    fn from_iter<T: IntoIterator<Item = Value<'gc>>>(iter: T) -> Self {
        ArrayVec {
            inner: Vec::from_iter(iter),
        }
    }
}

impl<'gc> Extend<Value<'gc>> for ArrayVec<'gc> {
    #[inline]
    fn extend<I: IntoIterator<Item = Value<'gc>>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}
