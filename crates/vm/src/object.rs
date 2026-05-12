use std::{
    cell::{Ref, RefMut},
    hash,
};

use gc_arena::{Collect, Gc, Mutation, RefLock};
use thiserror::Error;

use crate::{
    string::{String, StringMap},
    value::Value,
};

#[derive(Debug, Error)]
#[error("new object parent would create a cycle")]
pub struct CyclicObjectParent;

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct Object<'gc>(Gc<'gc, ObjectInner<'gc>>);

pub type ObjectInner<'gc> = RefLock<ObjectState<'gc>>;

#[derive(Debug, Default, Collect)]
#[collect(no_drop)]
pub struct ObjectState<'gc> {
    pub map: StringMap<'gc, Value<'gc>>,
    pub parent: Option<Object<'gc>>,
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

impl<'gc> Object<'gc> {
    pub fn new(mc: &Mutation<'gc>) -> Self {
        Self(Gc::new(mc, Default::default()))
    }

    pub fn from_iter(
        mc: &Mutation<'gc>,
        iter: impl Iterator<Item = (String<'gc>, Value<'gc>)>,
    ) -> Self {
        Self(Gc::new(
            mc,
            RefLock::new(ObjectState {
                map: iter.collect(),
                parent: None,
            }),
        ))
    }

    pub fn from_object_state(mc: &Mutation<'gc>, object_state: ObjectState<'gc>) -> Self {
        Self(Gc::new(mc, RefLock::new(object_state)))
    }

    #[inline]
    pub fn from_inner(inner: Gc<'gc, ObjectInner<'gc>>) -> Self {
        Self(inner)
    }

    #[inline]
    pub fn into_inner(self) -> Gc<'gc, ObjectInner<'gc>> {
        self.0
    }

    /// Get a value from this object or any parent object.
    pub fn get(self, key: String<'gc>) -> Option<Value<'gc>> {
        let mut state = self.0;
        loop {
            let s = state.borrow();
            if let Some(v) = s.map.get(&key).copied() {
                return Some(v);
            }

            if let Some(parent) = s.parent {
                state = parent.0;
            } else {
                return None;
            }
        }
    }

    /// Set a value in *this* object.
    ///
    /// If a value exists in a parent, that value will not be changed and a different value will be
    /// inserted into this object, overriding it.
    pub fn set(
        self,
        mc: &Mutation<'gc>,
        key: String<'gc>,
        value: impl Into<Value<'gc>>,
    ) -> Option<Value<'gc>> {
        self.0.borrow_mut(mc).map.insert(key, value.into())
    }

    pub fn remove(self, mc: &Mutation<'gc>, key: String<'gc>) -> Option<Value<'gc>> {
        self.0.borrow_mut(mc).map.remove(&key)
    }

    pub fn parent(self) -> Option<Object<'gc>> {
        self.0.borrow().parent
    }

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

        self.0.borrow_mut(mc).parent = new_parent;

        Ok(())
    }

    pub fn borrow(&self) -> Ref<'_, ObjectState<'gc>> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self, mc: &Mutation<'gc>) -> RefMut<'_, ObjectState<'gc>> {
        self.0.borrow_mut(mc)
    }
}
