use std::{
    cell::{Ref, RefMut},
    cmp,
    collections::BinaryHeap,
    convert::Infallible,
    sync::atomic,
};

use fabricator_vm as vm;
use gc_arena::{Collect, Gc, Mutation, RefLock, Rootable, barrier};

use crate::util::MagicExt as _;

#[derive(Collect)]
#[collect(no_drop)]
pub struct DsPriority<'gc> {
    inner: RefLock<BinaryHeap<Entry<'gc>>>,
    counter: i64,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct Entry<'gc> {
    pub priority: f64,
    pub value: vm::Value<'gc>,
}

impl<'gc> PartialEq for Entry<'gc> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl<'gc> Eq for Entry<'gc> {}

impl<'gc> PartialOrd for Entry<'gc> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<'gc> Ord for Entry<'gc> {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.priority.total_cmp(&other.priority)
    }
}

impl<'gc> DsPriority<'gc> {
    pub fn new() -> Self {
        static COUNTER: atomic::AtomicI64 = atomic::AtomicI64::new(0);
        let counter = COUNTER.fetch_add(1, atomic::Ordering::Relaxed);

        Self {
            inner: Default::default(),
            counter,
        }
    }

    pub fn into_userdata(self, ctx: vm::Context<'gc>) -> vm::UserData<'gc> {
        #[derive(Collect)]
        #[collect(require_static)]
        struct DsPriorityMethods;

        impl<'gc> vm::UserDataMethods<'gc> for DsPriorityMethods {
            fn coerce_integer(&self, ud: vm::UserData<'gc>, _ctx: vm::Context<'gc>) -> Option<i64> {
                Some(DsPriority::downcast(ud).unwrap().counter)
            }
        }

        #[derive(Collect)]
        #[collect(no_drop)]
        struct DsPriorityMethodsSingleton<'gc>(Gc<'gc, dyn vm::UserDataMethods<'gc>>);

        impl<'gc> vm::Singleton<'gc> for DsPriorityMethodsSingleton<'gc> {
            fn create(ctx: vm::Context<'gc>) -> Self {
                let methods = Gc::new(&ctx, DsPriorityMethods);
                DsPriorityMethodsSingleton(
                    gc_arena::unsize!(methods => dyn vm::UserDataMethods<'gc>),
                )
            }
        }

        let methods = ctx
            .singleton::<Rootable![DsPriorityMethodsSingleton<'_>]>()
            .0;
        let ud = vm::UserData::new::<Rootable![DsPriority<'_>]>(&ctx, self);
        ud.set_methods(&ctx, Some(methods));
        ud
    }

    #[inline]
    pub fn downcast(ud: vm::UserData<'gc>) -> Result<&'gc DsPriority<'gc>, vm::BadUserDataType> {
        ud.downcast::<Rootable![DsPriority<'_>]>()
    }

    #[inline]
    pub fn downcast_write(
        mc: &Mutation<'gc>,
        ud: vm::UserData<'gc>,
    ) -> Result<&'gc barrier::Write<DsPriority<'gc>>, vm::BadUserDataType> {
        ud.downcast_write::<Rootable![DsPriority<'_>]>(mc)
    }

    #[inline]
    pub fn borrow(&self) -> Ref<'_, BinaryHeap<Entry<'gc>>> {
        self.inner.borrow()
    }

    #[inline]
    pub fn borrow_mut(this: &barrier::Write<Self>) -> RefMut<'_, BinaryHeap<Entry<'gc>>> {
        let inner = barrier::field!(this, DsPriority, inner);
        inner.unlock().borrow_mut()
    }
}

pub fn ds_priority_create<'gc>(
    ctx: vm::Context<'gc>,
    (): (),
) -> Result<vm::UserData<'gc>, Infallible> {
    Ok(DsPriority::new().into_userdata(ctx))
}

/// Adds a new entry to a ds priority queue with a given priority.
pub fn ds_priority_add<'gc>(
    ctx: vm::Context<'gc>,
    (ds_priority_queue, value, priority): (vm::UserData<'gc>, vm::Value<'gc>, f64),
) -> Result<(), vm::user_data::BadUserDataType> {
    let ds_priority = DsPriority::downcast_write(&ctx, ds_priority_queue)?;
    let mut binary_heap = DsPriority::borrow_mut(ds_priority);
    binary_heap.push(Entry { priority, value });

    Ok(())
}

/// Clears a ds priority.
pub fn ds_priority_clear<'gc>(
    ctx: vm::Context<'gc>,
    ds_priority_queue: vm::UserData<'gc>,
) -> Result<(), vm::user_data::BadUserDataType> {
    let ds_priority = DsPriority::downcast_write(&ctx, ds_priority_queue)?;
    DsPriority::borrow_mut(ds_priority).clear();

    Ok(())
}

/// Clears the priority list. Since all `ds_` structures are garbage collected in
/// `fabricator`, simply stop referring to the priority list and it will be GCed.
pub fn ds_priority_destroy<'gc>(
    ctx: vm::Context<'gc>,
    ds_priority_queue: vm::UserData<'gc>,
) -> Result<(), vm::user_data::BadUserDataType> {
    ds_priority_clear(ctx, ds_priority_queue)
}

/// Gets the size of the [`BinaryHeap`](std::collections::BinaryHeap).
pub fn ds_priority_size<'gc>(
    _ctx: vm::Context<'gc>,
    ds_priority_queue: vm::UserData<'gc>,
) -> Result<i64, vm::user_data::BadUserDataType> {
    let ds_priority = DsPriority::downcast(ds_priority_queue)?;
    Ok(ds_priority.borrow().len() as i64)
}

/// Returns the maximum entry in the priority list, removing it from the list
/// in the process, and returning the entry. We do not return the priority that it had.
pub fn ds_priority_delete_max<'gc>(
    ctx: vm::Context<'gc>,
    ds_priority_queue: vm::UserData<'gc>,
) -> Result<Option<vm::Value<'gc>>, vm::user_data::BadUserDataType> {
    let ds_priority = DsPriority::downcast_write(&ctx, ds_priority_queue)?;
    let mut ds_priority = DsPriority::borrow_mut(ds_priority);
    let entry = ds_priority.pop();
    Ok(entry.map(|v| v.value))
}

pub fn ds_priority_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_callback(ctx, "ds_priority_create", ds_priority_create);
    lib.insert_callback(ctx, "ds_priority_add", ds_priority_add);
    lib.insert_callback(ctx, "ds_priority_clear", ds_priority_clear);
    lib.insert_callback(ctx, "ds_priority_destroy", ds_priority_destroy);
    lib.insert_callback(ctx, "ds_priority_size", ds_priority_size);
    lib.insert_callback(ctx, "ds_priority_delete_max", ds_priority_delete_max);
}
