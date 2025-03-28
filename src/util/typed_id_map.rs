use std::marker::PhantomData;

use super::id_map;

pub use super::id_map::{Generation, Index};

#[doc(hidden)]
pub trait Id {
    fn from_id(id: id_map::Id) -> Self;
    fn into_id(self) -> id_map::Id;
}

#[doc(hidden)]
#[macro_export]
macro_rules! __new_id_type {
    ( $(#[$outer:meta])* $vis:vis struct $name:ident; $($rest:tt)* ) => {
        $(#[$outer])*
        #[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
        #[repr(transparent)]
        $vis struct $name($crate::util::id_map::Id);

        impl $name {
            #[inline]
            pub fn index(&self) -> $crate::util::typed_id_map::Index {
                self.0.index()
            }

            #[inline]
            pub fn generation(&self) -> ::std::num::NonZero<$crate::util::typed_id_map::Generation> {
                self.0.generation()
            }
        }

        impl $crate::util::typed_id_map::Id for $name {
            #[inline]
            fn from_id(id: $crate::util::id_map::Id) -> Self {
                $name(id)
            }

            #[inline]
             fn into_id(self) -> $crate::util::id_map::Id {
                self.0
            }
        }

        $crate::__new_id_type!($($rest)*);
    };

    () => {}
}
pub use crate::__new_id_type as new_id_type;

pub struct IdMap<I, V> {
    map: id_map::IdMap<V>,
    _marker: PhantomData<I>,
}

impl<I, V> Default for IdMap<I, V> {
    fn default() -> Self {
        Self {
            map: Default::default(),
            _marker: PhantomData,
        }
    }
}

impl<I: Id, V> IdMap<I, V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, value: V) -> I {
        I::from_id(self.map.insert(value))
    }

    pub fn remove(&mut self, id: I) -> Option<V> {
        self.map.remove(id.into_id())
    }

    pub fn contains(&mut self, id: I) -> bool {
        self.map.contains(id.into_id())
    }

    pub fn get(&self, id: I) -> Option<&V> {
        self.map.get(id.into_id())
    }

    pub fn get_mut(&mut self, id: I) -> Option<&mut V> {
        self.map.get_mut(id.into_id())
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn index_upper_bound(&self) -> Index {
        self.map.index_upper_bound()
    }
}

pub struct SecondaryMap<I: Id, V> {
    map: id_map::SecondaryMap<V>,
    _marker: PhantomData<I>,
}

impl<I: Id, V> Default for SecondaryMap<I, V> {
    fn default() -> Self {
        Self {
            map: Default::default(),
            _marker: PhantomData,
        }
    }
}

impl<I: Id, V> SecondaryMap<I, V> {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn insert(&mut self, key: I, val: V) -> Option<(I, V)> {
        self.map
            .insert(key.into_id(), val)
            .map(|(i, v)| (I::from_id(i), v))
    }

    pub fn remove(&mut self, key: I) -> Option<V> {
        self.map.remove(key.into_id())
    }

    pub fn get(&self, key: I) -> Option<&V> {
        self.map.get(key.into_id())
    }

    pub fn get_mut(&mut self, key: I) -> Option<&mut V> {
        self.map.get_mut(key.into_id())
    }
}
