use std::{
    borrow::Borrow,
    fmt, hash,
    ops::{self, Deref},
    sync::Arc,
};

use gc_arena::{
    Collect, Gc, GcWeak, Mutation, barrier::Unlock as _, collect::Trace, lock::RefLock,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// A shared string with 'static lifetime.
///
/// Can be backed by either an `Arc<str>` or a `&'static str`.
#[derive(Clone, Collect)]
#[collect(require_static)]
pub struct SharedStr(SharedStrInner);

#[derive(Clone, Collect)]
#[collect(require_static)]
enum SharedStrInner {
    Arc(Arc<str>),
    Static(&'static str),
}

impl From<Arc<str>> for SharedStr {
    #[inline]
    fn from(s: Arc<str>) -> Self {
        SharedStr(SharedStrInner::Arc(s))
    }
}

impl From<&'static str> for SharedStr {
    #[inline]
    fn from(s: &'static str) -> Self {
        SharedStr(SharedStrInner::Static(s))
    }
}

impl PartialEq for SharedStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for SharedStr {}

impl hash::Hash for SharedStr {
    #[inline]
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl fmt::Display for SharedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl fmt::Debug for SharedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl ops::Deref for SharedStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for SharedStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl SharedStr {
    #[inline]
    pub fn new(s: &str) -> SharedStr {
        SharedStr(SharedStrInner::Arc(s.into()))
    }

    #[inline]
    pub fn from_arc(s: Arc<str>) -> SharedStr {
        SharedStr(SharedStrInner::Arc(s))
    }

    #[inline]
    pub fn from_static(s: &'static str) -> SharedStr {
        SharedStr(SharedStrInner::Static(s))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        match &self.0 {
            SharedStrInner::Arc(s) => s.as_ref(),
            &SharedStrInner::Static(s) => s,
        }
    }
}

impl Borrow<str> for SharedStr {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

/// A unique, garbage collected reference to a string.
///
/// Because the string is guaranteed to be unique for a specific interpreter, hashing and equality
/// checks are done by *GC pointer value* without needing to reference the string itself.
#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct String<'gc>(Gc<'gc, SharedStr>);

impl<'gc> PartialEq for String<'gc> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Gc::as_ptr(self.0) == Gc::as_ptr(other.0)
    }
}

impl<'gc> Eq for String<'gc> {}

impl<'gc> hash::Hash for String<'gc> {
    #[inline]
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        Gc::as_ptr(self.0).hash(state)
    }
}

impl<'gc> fmt::Display for String<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'gc> fmt::Debug for String<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'gc> String<'gc> {
    #[inline]
    pub fn from_inner(inner: Gc<'gc, SharedStr>) -> Self {
        Self(inner)
    }

    #[inline]
    pub fn into_inner(self) -> Gc<'gc, SharedStr> {
        self.0
    }

    #[inline]
    pub fn as_str(self) -> &'gc str {
        self.0.as_ref().as_str()
    }

    #[inline]
    pub fn as_shared(self) -> &'gc SharedStr {
        self.0.as_ref()
    }
}

impl<'gc> AsRef<str> for String<'gc> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl<'gc> Deref for String<'gc> {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

pub type StringMap<'gc, V> = FxHashMap<String<'gc>, V>;
pub type StringSet<'gc> = FxHashSet<String<'gc>>;

struct InternedStringsInner<'gc>(RefLock<FxHashMap<SharedStr, GcWeak<'gc, SharedStr>>>);

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct InternedStrings<'gc>(Gc<'gc, InternedStringsInner<'gc>>);

unsafe impl<'gc> Collect<'gc> for InternedStringsInner<'gc> {
    const NEEDS_TRACE: bool = true;

    fn trace<T: Trace<'gc>>(&self, cc: &mut T) {
        // SAFETY: No new Gc pointers are adopted or reparented.
        let mut strings = unsafe { self.0.unlock_unchecked() }.borrow_mut();
        strings.retain(|_, s| !s.is_dropped());
        strings.trace(cc);
    }
}

impl<'gc> InternedStrings<'gc> {
    pub(crate) fn new(mc: &Mutation<'gc>) -> Self {
        Self(Gc::new(mc, InternedStringsInner(Default::default())))
    }

    pub fn intern(self, mc: &Mutation<'gc>, s: &str) -> String<'gc> {
        self.intern_shared(mc, s, || SharedStr(SharedStrInner::Arc(s.into())))
    }

    pub fn intern_static(self, mc: &Mutation<'gc>, s: &'static str) -> String<'gc> {
        self.intern_shared(mc, s, || s.into())
    }

    /// For correctness, `make_shared` *must* return a shared version of the same string `s`,
    /// otherwise, the [`String`] uniqueness guarantee may not be upheld.
    fn intern_shared(
        self,
        mc: &Mutation<'gc>,
        s: &str,
        make_shared: impl FnOnce() -> SharedStr,
    ) -> String<'gc> {
        // SAFETY: If a new string is added, we call an appropriate write barrier.
        let mut strings = unsafe { self.0.0.unlock_unchecked() }.borrow_mut();

        if let Some(shared) = strings.get(s) {
            if let Some(string) = shared.upgrade(mc) {
                return String(string);
            }
        }

        let shared = make_shared();
        let string = Gc::new(&mc, shared.clone());
        let weak_string = Gc::downgrade(string);

        // SAFETY: We are adopting a new weak child, so we need an appropriate barrier.
        mc.forward_barrier_weak(Some(Gc::erase(self.0)), GcWeak::erase(weak_string));
        strings.insert(shared, weak_string);

        String(string)
    }
}
