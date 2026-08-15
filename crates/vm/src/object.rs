use std::{
    cell::{Ref, RefMut},
    collections::hash_map,
    hash, iter,
};

use gc_arena::{Collect, Gc, Lock, Mutation, RefLock, barrier};
use thiserror::Error;

use crate::{
    conversion::{FromValue, IntoValue},
    error::RuntimeError,
    interpreter::Context,
    string::{String, StringMap},
    value::Value,
};

#[derive(Debug, Error)]
#[error("new object parent would create a cycle")]
pub struct CyclicObjectParent;

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct Object<'gc>(Gc<'gc, ObjectInner<'gc>>);

#[derive(Debug, Collect)]
#[collect(no_drop)]
pub struct ObjectInner<'gc> {
    map: RefLock<ObjectMap<'gc>>,
    parent: Lock<Option<Object<'gc>>>,
}

impl<'gc> PartialEq for Object<'gc> {
    fn eq(&self, other: &Self) -> bool {
        Gc::ptr_eq(self.0, other.0)
    }
}

impl<'gc> Eq for Object<'gc> {}

impl<'gc> hash::Hash for Object<'gc> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        Gc::as_ptr(self.0).hash(state)
    }
}

#[derive(Debug, Error)]
#[error("`Object` is already borrowed mutably")]
pub struct ObjectBorrowError;

#[derive(Debug, Error)]
#[error("`Object` is already borrowed")]
pub struct ObjectBorrowMutError;

impl<'gc> Object<'gc> {
    #[inline]
    pub fn new(mc: &Mutation<'gc>) -> Self {
        Self(Gc::new(
            mc,
            ObjectInner {
                map: RefLock::new(ObjectMap {
                    inner: StringMap::default(),
                }),
                parent: Lock::new(None),
            },
        ))
    }

    #[inline]
    pub fn with_parts(
        mc: &Mutation<'gc>,
        map: ObjectMap<'gc>,
        parent: Option<Object<'gc>>,
    ) -> Self {
        Self(Gc::new(
            mc,
            ObjectInner {
                map: RefLock::new(map),
                parent: Lock::new(parent),
            },
        ))
    }

    #[inline]
    pub fn from_iter(
        mc: &Mutation<'gc>,
        iter: impl IntoIterator<Item = (String<'gc>, Value<'gc>)>,
    ) -> Self {
        Self::with_parts(mc, ObjectMap::from_iter(iter), None)
    }

    #[inline]
    pub fn from_inner(inner: Gc<'gc, ObjectInner<'gc>>) -> Self {
        Self(inner)
    }

    #[inline]
    pub fn into_inner(self) -> Gc<'gc, ObjectInner<'gc>> {
        self.0
    }

    /// Get a value from this object, or if it is not found, any transitive parent object.
    ///
    /// # Panics
    ///
    /// Panics if the inner `ObjectMap` is borrowed mutably, and may panic if the inner `ObjectMap`
    /// *of any parent* is borrowed mutably.
    #[inline]
    pub fn find(&self, key: String<'gc>) -> Option<Value<'gc>> {
        self.try_find(key).unwrap()
    }

    /// A convenience method to call [`Object::find`] on a static string key.
    ///
    /// # Panics
    ///
    /// Panics if the inner `ObjectMap` is borrowed mutably, and may panic if the inner `ObjectMap`
    /// *of any parent* is borrowed mutably.
    #[inline]
    pub fn find_field(&self, ctx: Context<'gc>, key: &'static str) -> Option<Value<'gc>> {
        self.try_find_field(ctx, key).unwrap()
    }

    /// Get a value from this object, or if it is not found, any transitive parent object.
    #[inline]
    pub fn try_find(&self, key: String<'gc>) -> Result<Option<Value<'gc>>, ObjectBorrowError> {
        if let Some(value) = self.try_borrow()?.get(key) {
            return Ok(Some(value));
        }

        let mut parent = self.0.parent.get();
        while let Some(object) = parent {
            if let Some(v) = object.try_borrow()?.get(key) {
                return Ok(Some(v));
            }
            parent = object.0.parent.get();
        }
        Ok(None)
    }

    /// A convenience method to call [`Object::try_find`] on a static string key.
    #[inline]
    pub fn try_find_field(
        &self,
        ctx: Context<'gc>,
        key: &'static str,
    ) -> Result<Option<Value<'gc>>, ObjectBorrowError> {
        self.try_find(ctx.intern_static(key))
    }

    /// Return the parent object of this object, if one is set.
    #[inline]
    pub fn parent(self) -> Option<Object<'gc>> {
        self.0.parent.get()
    }

    /// Set the parent of this object.
    ///
    /// If `new_parent` is `Some`, this will walk the chain of all parents to make sure that
    /// `new_parent` does not have this object as its parent already. If it does, this will return
    /// `CyclicObjectParent`.
    pub fn set_parent(
        self,
        mc: &Mutation<'gc>,
        new_parent: Option<Object<'gc>>,
    ) -> Result<(), CyclicObjectParent> {
        // Ensure that if a new parent is given, this object is not anywhere within its ancestry.
        //
        // If it was, this would create a cyclic object parent relationship.
        if let Some(new_parent) = new_parent {
            let mut cur_parent = new_parent;
            loop {
                if cur_parent == self {
                    return Err(CyclicObjectParent);
                }

                if let Some(parent) = cur_parent.parent() {
                    cur_parent = parent;
                } else {
                    break;
                }
            }
        }

        let inner = Gc::write(mc, self.0);
        let parent = barrier::field!(inner, ObjectInner, parent);
        parent.unlock().set(new_parent);

        Ok(())
    }

    #[inline]
    pub fn borrow(&self) -> Ref<'_, ObjectMap<'gc>> {
        self.try_borrow().unwrap()
    }

    #[inline]
    pub fn borrow_mut(&self, mc: &Mutation<'gc>) -> RefMut<'_, ObjectMap<'gc>> {
        self.try_borrow_mut(mc).unwrap()
    }

    #[inline]
    pub fn try_borrow(&self) -> Result<Ref<'_, ObjectMap<'gc>>, ObjectBorrowError> {
        self.0.map.try_borrow().map_err(|_| ObjectBorrowError)
    }

    #[inline]
    pub fn try_borrow_mut(
        &self,
        mc: &Mutation<'gc>,
    ) -> Result<RefMut<'_, ObjectMap<'gc>>, ObjectBorrowMutError> {
        let inner = Gc::write(mc, self.0);
        barrier::field!(inner, ObjectInner, map)
            .unlock()
            .try_borrow_mut()
            .map_err(|_| ObjectBorrowMutError)
    }
}

#[derive(Debug, Default, Collect)]
#[collect(no_drop)]
pub struct ObjectMap<'gc> {
    inner: StringMap<'gc, Value<'gc>>,
}

impl<'gc> IntoValue<'gc> for ObjectMap<'gc> {
    #[inline]
    fn into_value(self, ctx: Context<'gc>) -> Value<'gc> {
        Object::with_parts(&ctx, self, None).into()
    }
}

impl<'gc> ObjectMap<'gc> {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get a value from *this* object only.
    #[inline]
    pub fn get(&self, key: String<'gc>) -> Option<Value<'gc>> {
        self.inner.get(&key).copied()
    }

    /// Set a value in *this* object.
    ///
    /// If a value exists in a parent, that value will not be changed and a different value will be
    /// inserted into this object, overriding it.
    ///
    /// Returns the previously set value in this object, if one was present.
    #[inline]
    pub fn set(&mut self, key: String<'gc>, value: impl Into<Value<'gc>>) -> Option<Value<'gc>> {
        self.inner.insert(key, value.into())
    }

    /// Remove a value from *this* object only.
    #[inline]
    pub fn remove(&mut self, key: String<'gc>) -> Option<Value<'gc>> {
        self.inner.remove(&key)
    }

    /// A convenience method to call [`ObjectMap::get`] on a static string key with automatic type
    /// conversion of the value.
    ///
    /// If a key is missing, the value will be converted from `Value::Undefined`.
    #[inline]
    pub fn get_field<V: FromValue<'gc>>(
        &self,
        ctx: Context<'gc>,
        key: &'static str,
    ) -> Result<V, RuntimeError> {
        V::from_value(ctx, self.get(ctx.intern_static(key)).unwrap_or_default())
    }

    /// A convenience method to call [`ObjectMap::set`] on a static string key with automatic type
    /// conversion of the key.
    #[inline]
    pub fn set_field(
        &mut self,
        ctx: Context<'gc>,
        key: &'static str,
        value: impl IntoValue<'gc>,
    ) -> Option<Value<'gc>> {
        self.set(ctx.intern_static(key), value.into_value(ctx))
    }

    #[inline]
    pub fn keys(&self) -> iter::Copied<hash_map::Keys<'_, String<'gc>, Value<'gc>>> {
        self.inner.keys().copied()
    }

    #[inline]
    pub fn values(&self) -> iter::Copied<hash_map::Values<'_, String<'gc>, Value<'gc>>> {
        self.inner.values().copied()
    }

    #[inline]
    pub fn values_mut(&mut self) -> hash_map::ValuesMut<'_, String<'gc>, Value<'gc>> {
        self.inner.values_mut()
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
    pub fn drain(&mut self) -> hash_map::Drain<'_, String<'gc>, Value<'gc>> {
        self.inner.drain()
    }
}

pub struct CopyKey<I>(I);

impl<'a, K, V, I> Iterator for CopyKey<I>
where
    K: Copy + 'a,
    I: Iterator<Item = (&'a K, V)>,
{
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        let (k, v) = self.0.next()?;
        Some((*k, v))
    }
}

impl<'a, K, V, I> ExactSizeIterator for CopyKey<I>
where
    K: Copy + 'a,
    I: ExactSizeIterator<Item = (&'a K, V)>,
{
    fn len(&self) -> usize {
        self.0.len()
    }
}

pub struct CopyValue<I>(I);

impl<'a, K, V, I> Iterator for CopyValue<I>
where
    V: Copy + 'a,
    I: Iterator<Item = (K, &'a V)>,
{
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        let (k, v) = self.0.next()?;
        Some((k, *v))
    }
}

impl<'a, K, V, I> ExactSizeIterator for CopyValue<I>
where
    V: Copy + 'a,
    I: ExactSizeIterator<Item = (K, &'a V)>,
{
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<'gc, 'a> IntoIterator for &'a ObjectMap<'gc> {
    type Item = (String<'gc>, Value<'gc>);
    type IntoIter = CopyKey<CopyValue<hash_map::Iter<'a, String<'gc>, Value<'gc>>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        CopyKey(CopyValue(self.inner.iter()))
    }
}

impl<'gc, 'a> IntoIterator for &'a mut ObjectMap<'gc> {
    type Item = (String<'gc>, &'a mut Value<'gc>);
    type IntoIter = CopyKey<hash_map::IterMut<'a, String<'gc>, Value<'gc>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        CopyKey(self.inner.iter_mut())
    }
}

impl<'gc> FromIterator<(String<'gc>, Value<'gc>)> for ObjectMap<'gc> {
    #[inline]
    fn from_iter<T: IntoIterator<Item = (String<'gc>, Value<'gc>)>>(iter: T) -> Self {
        ObjectMap {
            inner: StringMap::from_iter(iter),
        }
    }
}

impl<'gc> Extend<(String<'gc>, Value<'gc>)> for ObjectMap<'gc> {
    #[inline]
    fn extend<I: IntoIterator<Item = (String<'gc>, Value<'gc>)>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}
