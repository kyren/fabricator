use std::{marker::PhantomData, num::NonZero, ops};

pub use crate::id_map::{self, Generation, Index};

pub trait Id {
    fn index(&self) -> Index;
    fn generation(&self) -> NonZero<Generation>;

    #[doc(hidden)]
    fn from_id(id: id_map::Id) -> Self;
    #[doc(hidden)]
    fn into_id(self) -> id_map::Id;
}

#[doc(hidden)]
#[macro_export]
macro_rules! __new_id_type {
    ( $(#[$outer:meta])* $vis:vis struct $name:ident; $($rest:tt)* ) => {
        $(#[$outer])*
        #[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
        #[repr(transparent)]
        $vis struct $name($crate::id_map::Id);

        impl $name {
            #[inline]
            pub fn index(&self) -> $crate::typed_id_map::Index {
                self.0.index()
            }

            #[inline]
            pub fn generation(&self) -> ::std::num::NonZero<$crate::typed_id_map::Generation> {
                self.0.generation()
            }
        }

        impl $crate::typed_id_map::Id for $name {
            #[inline]
            fn index(&self) -> $crate::typed_id_map::Index {
                self.index()
            }

            #[inline]
            fn generation(&self) -> ::std::num::NonZero<$crate::typed_id_map::Generation> {
                self.generation()
            }

            #[inline]
            fn from_id(id: $crate::id_map::Id) -> Self {
                $name(id)
            }

            #[inline]
             fn into_id(self) -> $crate::id_map::Id {
                self.0
            }
        }

        $crate::__new_id_type!($($rest)*);
    };

    () => {}
}

#[doc(inline)]
pub use crate::__new_id_type as new_id_type;

#[derive(Clone)]
pub struct IdMap<I, V> {
    map: id_map::IdMap<V>,
    _marker: PhantomData<I>,
}

impl<I, V> Default for IdMap<I, V> {
    #[inline]
    fn default() -> Self {
        Self {
            map: Default::default(),
            _marker: PhantomData,
        }
    }
}

impl<I: Id, V> IdMap<I, V> {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn insert(&mut self, value: V) -> I {
        I::from_id(self.map.insert(value))
    }

    #[inline]
    pub fn insert_with_id(&mut self, value: impl FnOnce(I) -> V) -> I {
        I::from_id(self.map.insert_with_id(move |id| value(I::from_id(id))))
    }

    #[inline]
    pub fn remove(&mut self, id: I) -> Option<V> {
        self.map.remove(id.into_id())
    }

    #[inline]
    pub fn contains(&self, id: I) -> bool {
        self.map.contains(id.into_id())
    }

    #[inline]
    pub fn get(&self, id: I) -> Option<&V> {
        self.map.get(id.into_id())
    }

    #[inline]
    pub fn get_mut(&mut self, id: I) -> Option<&mut V> {
        self.map.get_mut(id.into_id())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    #[inline]
    pub fn index_upper_bound(&self) -> Index {
        self.map.index_upper_bound()
    }

    #[inline]
    pub fn id_for_index(&self, index: Index) -> Option<I> {
        Some(I::from_id(self.map.id_for_index(index)?))
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (I, &V)> + '_ {
        self.map.iter().map(|(id, v)| (I::from_id(id), v))
    }

    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (I, &mut V)> + '_ {
        self.map.iter_mut().map(|(id, v)| (I::from_id(id), v))
    }

    #[inline]
    pub fn ids(&self) -> impl Iterator<Item = I> + '_ {
        self.map.ids().map(I::from_id)
    }

    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.map.values()
    }

    #[inline]
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> + '_ {
        self.map.values_mut()
    }

    #[inline]
    pub fn retain(&mut self, mut f: impl FnMut(I, &mut V) -> bool) {
        self.map.retain(move |id, val| f(I::from_id(id), val))
    }

    #[inline]
    pub fn map_value<V2>(self, f: impl FnMut(V) -> V2) -> IdMap<I, V2> {
        IdMap {
            map: self.map.map_value(f),
            _marker: PhantomData,
        }
    }
}

impl<I: Id, V> ops::Index<I> for IdMap<I, V> {
    type Output = V;

    #[inline]
    #[track_caller]
    fn index(&self, id: I) -> &V {
        self.get(id).expect("no such id in `IdMap`")
    }
}

impl<I: Id, V> ops::IndexMut<I> for IdMap<I, V> {
    #[inline]
    #[track_caller]
    fn index_mut(&mut self, id: I) -> &mut Self::Output {
        self.get_mut(id).expect("no such id in `IdMap`")
    }
}

#[derive(Debug, Clone)]
pub struct SecondaryMap<I: Id, V> {
    map: id_map::SecondaryMap<V>,
    _marker: PhantomData<I>,
}

impl<I: Id, V> Default for SecondaryMap<I, V> {
    #[inline]
    fn default() -> Self {
        Self {
            map: Default::default(),
            _marker: PhantomData,
        }
    }
}

impl<I: Id, V> FromIterator<(I, V)> for SecondaryMap<I, V> {
    fn from_iter<T: IntoIterator<Item = (I, V)>>(iter: T) -> Self {
        let mut map = Self::new();
        for (i, v) in iter {
            map.insert(i, v);
        }
        map
    }
}

impl<I: Id, V> SecondaryMap<I, V> {
    #[inline]
    pub fn new() -> Self {
        Default::default()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.map.clear();
    }

    #[inline]
    pub fn insert(&mut self, key: I, val: V) -> Option<(I, V)> {
        self.map
            .insert(key.into_id(), val)
            .map(|(i, v)| (I::from_id(i), v))
    }

    #[inline]
    pub fn remove(&mut self, key: I) -> Option<V> {
        self.map.remove(key.into_id())
    }

    #[inline]
    pub fn contains(&mut self, id: I) -> bool {
        self.map.contains(id.into_id())
    }

    #[inline]
    pub fn get(&self, key: I) -> Option<&V> {
        self.map.get(key.into_id())
    }

    #[inline]
    pub fn get_mut(&mut self, key: I) -> Option<&mut V> {
        self.map.get_mut(key.into_id())
    }

    #[inline]
    pub fn get_or_insert_with(&mut self, key: I, f: impl FnOnce() -> V) -> &mut V {
        self.map.get_or_insert_with(key.into_id(), f)
    }

    #[inline]
    pub fn get_or_insert_default(&mut self, key: I) -> &mut V
    where
        V: Default,
    {
        self.map.get_or_insert_default(key.into_id())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    #[inline]
    pub fn index_upper_bound(&self) -> Index {
        self.map.index_upper_bound()
    }

    #[inline]
    pub fn id_for_index(&self, index: Index) -> Option<I> {
        Some(I::from_id(self.map.id_for_index(index)?))
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (I, &V)> + '_ {
        self.map.iter().map(|(id, v)| (I::from_id(id), v))
    }

    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (I, &mut V)> + '_ {
        self.map.iter_mut().map(|(id, v)| (I::from_id(id), v))
    }

    #[inline]
    pub fn into_iter(self) -> impl Iterator<Item = (I, V)> {
        self.map.into_iter().map(|(id, v)| (I::from_id(id), v))
    }

    #[inline]
    pub fn ids(&self) -> impl Iterator<Item = I> + '_ {
        self.iter().map(|(id, _)| id)
    }

    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.iter().map(|(_, v)| v)
    }

    #[inline]
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> + '_ {
        self.iter_mut().map(|(_, v)| v)
    }

    #[inline]
    pub fn retain(&mut self, mut f: impl FnMut(I, &mut V) -> bool) {
        self.map.retain(move |id, val| f(I::from_id(id), val))
    }
}

impl<I: Id, V> ops::Index<I> for SecondaryMap<I, V> {
    type Output = V;

    #[inline]
    #[track_caller]
    fn index(&self, id: I) -> &V {
        self.get(id).expect("no such id in `SecondaryMap`")
    }
}
