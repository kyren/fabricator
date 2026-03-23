use std::{
    cell::{Ref, RefMut},
    convert::Infallible,
    sync::atomic,
};

use fabricator_vm as vm;
use gc_arena::{Collect, Gc, Mutation, RefLock, Rootable, barrier};

use crate::util::MagicExt as _;

#[derive(Collect)]
#[collect(no_drop)]
pub struct DsList<'gc> {
    inner: RefLock<Vec<vm::Value<'gc>>>,
    counter: i64,
}

impl<'gc> DsList<'gc> {
    pub fn new() -> Self {
        static COUNTER: atomic::AtomicI64 = atomic::AtomicI64::new(0);
        let counter = COUNTER.fetch_add(1, atomic::Ordering::Relaxed);

        Self {
            inner: RefLock::new(Vec::new()),
            counter,
        }
    }

    pub fn into_userdata(self, ctx: vm::Context<'gc>) -> vm::UserData<'gc> {
        #[derive(Collect)]
        #[collect(require_static)]
        struct DsListMethods;

        impl<'gc> vm::UserDataMethods<'gc> for DsListMethods {
            fn get_index(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                indexes: &[vm::Value<'gc>],
            ) -> Result<vm::Value<'gc>, vm::RuntimeError> {
                if indexes.len() != 1 {
                    return Err(vm::RuntimeError::msg("expected 1 index for ds_list"));
                }
                let i: usize = vm::FromValue::from_value(ctx, indexes[0])?;
                Ok(DsList::downcast(ud)
                    .unwrap()
                    .inner
                    .borrow()
                    .get(i)
                    .copied()
                    .unwrap_or_default())
            }

            fn set_index(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                indexes: &[vm::Value<'gc>],
                value: vm::Value<'gc>,
            ) -> Result<(), vm::RuntimeError> {
                if indexes.len() != 1 {
                    return Err(vm::RuntimeError::msg("expected 1 index for ds_list"));
                }
                let i: usize = vm::FromValue::from_value(ctx, indexes[0])?;
                let ds_list = DsList::downcast_write(&ctx, ud).unwrap();
                let inner = barrier::field!(ds_list, DsList, inner);
                let mut vec = inner.unlock().borrow_mut();
                if i >= vec.len() {
                    vec.resize(i + 1, vm::Value::Undefined);
                }
                vec[i] = value;
                Ok(())
            }

            fn coerce_integer(&self, ud: vm::UserData<'gc>, _ctx: vm::Context<'gc>) -> Option<i64> {
                Some(DsList::downcast(ud).unwrap().counter)
            }
        }

        #[derive(Collect)]
        #[collect(no_drop)]
        struct DsListMethodsSingleton<'gc>(Gc<'gc, dyn vm::UserDataMethods<'gc>>);

        impl<'gc> vm::Singleton<'gc> for DsListMethodsSingleton<'gc> {
            fn create(ctx: vm::Context<'gc>) -> Self {
                let methods = Gc::new(&ctx, DsListMethods);
                DsListMethodsSingleton(gc_arena::unsize!(methods => dyn vm::UserDataMethods<'gc>))
            }
        }

        let methods = ctx.singleton::<Rootable![DsListMethodsSingleton<'_>]>().0;
        let ud = vm::UserData::new::<Rootable![DsList<'_>]>(&ctx, self);
        ud.set_methods(&ctx, Some(methods));
        ud
    }

    #[inline]
    pub fn downcast(ud: vm::UserData<'gc>) -> Result<&'gc DsList<'gc>, vm::BadUserDataType> {
        ud.downcast::<Rootable![DsList<'_>]>()
    }

    #[inline]
    pub fn downcast_write(
        mc: &Mutation<'gc>,
        ud: vm::UserData<'gc>,
    ) -> Result<&'gc barrier::Write<DsList<'gc>>, vm::BadUserDataType> {
        ud.downcast_write::<Rootable![DsList<'_>]>(mc)
    }

    #[inline]
    pub fn borrow(&self) -> Ref<'_, Vec<vm::Value<'gc>>> {
        self.inner.borrow()
    }

    #[inline]
    pub fn borrow_mut(this: &barrier::Write<Self>) -> RefMut<'_, Vec<vm::Value<'gc>>> {
        let inner = barrier::field!(this, DsList, inner);
        inner.unlock().borrow_mut()
    }
}

pub fn ds_list_create<'gc>(ctx: vm::Context<'gc>, (): ()) -> Result<vm::UserData<'gc>, Infallible> {
    Ok(DsList::new().into_userdata(ctx))
}

pub fn ds_list_add<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let ds_list: vm::UserData = exec.stack().from_index(ctx, 0)?;
    let ds_list = DsList::downcast_write(&ctx, ds_list)?;
    let mut vec = DsList::borrow_mut(ds_list);
    vec.extend_from_slice(&exec.stack()[1..]);
    exec.stack().clear();
    Ok(())
}

pub fn ds_list_find_index<'gc>(
    _ctx: vm::Context<'gc>,
    (ds_list, value): (vm::UserData<'gc>, vm::Value<'gc>),
) -> Result<isize, vm::BadUserDataType> {
    let ds_list = DsList::downcast(ds_list)?;
    let index = ds_list
        .inner
        .borrow()
        .iter()
        .position(|&v| v == value)
        .map(|i| i as isize)
        .unwrap_or(-1);
    Ok(index)
}

pub fn ds_list_delete<'gc>(
    ctx: vm::Context<'gc>,
    (ds_list, index): (vm::UserData<'gc>, usize),
) -> Result<(), vm::RuntimeError> {
    let ds_list = DsList::downcast_write(&ctx, ds_list)?;
    let mut vec = DsList::borrow_mut(ds_list);
    if index >= vec.len() {
        return Err(vm::RuntimeError::msg(format!(
            "index {index} out of range of ds_list with length {}",
            vec.len()
        )));
    }
    vec.remove(index);
    Ok(())
}

pub fn ds_list_size<'gc>(
    _ctx: vm::Context<'gc>,
    ds_list: vm::UserData<'gc>,
) -> Result<isize, vm::RuntimeError> {
    let ds_list = DsList::downcast(ds_list)?;
    Ok(ds_list.borrow().len() as isize)
}

pub fn ds_list_clear<'gc>(
    ctx: vm::Context<'gc>,
    ds_list: vm::UserData<'gc>,
) -> Result<(), vm::BadUserDataType> {
    let ds_list = DsList::downcast_write(&ctx, ds_list)?;
    let mut vec = DsList::borrow_mut(ds_list);
    vec.clear();
    Ok(())
}

pub fn ds_list_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_callback(ctx, "ds_list_create", ds_list_create);
    lib.insert_exec_callback(ctx, "ds_list_add", ds_list_add);
    lib.insert_callback(ctx, "ds_list_find_index", ds_list_find_index);
    lib.insert_callback(ctx, "ds_list_delete", ds_list_delete);
    lib.insert_callback(ctx, "ds_list_size", ds_list_size);
    lib.insert_callback(ctx, "ds_list_clear", ds_list_clear);
}
