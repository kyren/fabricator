use std::marker::PhantomData;

use fabricator_util::typed_id_map;
use fabricator_vm as vm;
use gc_arena::{Collect, Gc, Rootable};

/// A wrapper around a `typed_id_map::Id` ID type.
///
/// Can be coerced to an integer in scripts, which yields the id's index.
#[derive(Debug, Copy, Clone)]
pub struct IdUserData<I> {
    pub id: I,
}

impl<I> IdUserData<I>
where
    I: typed_id_map::Id + 'static,
{
    pub fn new<'gc>(ctx: vm::Context<'gc>, id: I) -> vm::UserData<'gc> {
        struct Methods<I>(PhantomData<I>);

        impl<'gc, I> vm::UserDataMethods<'gc> for Methods<I>
        where
            I: typed_id_map::Id + 'static,
        {
            fn coerce_integer(&self, ud: vm::UserData<'gc>, _ctx: vm::Context<'gc>) -> Option<i64> {
                Some(ud.downcast_static::<IdUserData<I>>().unwrap().id.index() as i64)
            }
        }

        #[derive(Collect)]
        #[collect(no_drop, bound = "")]
        struct MethodsSingleton<'gc, I>(Gc<'gc, dyn vm::UserDataMethods<'gc>>, PhantomData<I>);

        impl<'gc, I> vm::Singleton<'gc> for MethodsSingleton<'gc, I>
        where
            I: typed_id_map::Id + 'static,
        {
            fn create(ctx: vm::Context<'gc>) -> Self {
                let methods = ctx.alloc_static(Methods::<I>(PhantomData));
                MethodsSingleton(
                    gc_arena::unsize!(methods => dyn vm::UserDataMethods<'gc>),
                    PhantomData,
                )
            }
        }

        let methods = ctx.singleton::<Rootable![MethodsSingleton<'_, I>]>().0;

        let userdata = vm::UserData::new_static(&ctx, IdUserData { id });
        userdata.set_methods(&ctx, Some(methods));

        userdata
    }

    pub fn downcast<'gc>(userdata: vm::UserData<'gc>) -> Result<&'gc Self, vm::BadUserDataType> {
        userdata.downcast_static::<IdUserData<I>>()
    }
}

/// A wrapper around a `typed_id_map::Id` ID type with an assigned name.
///
/// Can be coerced to an integer or string in scripts. Coercing to an integer yields the id's index,
/// coercing to a string yields the given name.
#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct NamedIdUserData<'gc, I> {
    #[collect(require_static)]
    pub id: I,
    pub name: vm::String<'gc>,
}

impl<'gc, I> NamedIdUserData<'gc, I>
where
    I: typed_id_map::Id + 'static,
{
    pub fn new(ctx: vm::Context<'gc>, id: I, name: vm::String<'gc>) -> vm::UserData<'gc> {
        struct Methods<I>(PhantomData<I>);

        impl<'gc, I> vm::UserDataMethods<'gc> for Methods<I>
        where
            I: typed_id_map::Id + 'static,
        {
            fn coerce_string(
                &self,
                ud: vm::UserData<'gc>,
                _ctx: vm::Context<'gc>,
            ) -> Option<vm::String<'gc>> {
                Some(
                    ud.downcast::<Rootable![NamedIdUserData<'_, I>]>()
                        .unwrap()
                        .name,
                )
            }

            fn coerce_integer(&self, ud: vm::UserData<'gc>, _ctx: vm::Context<'gc>) -> Option<i64> {
                Some(
                    ud.downcast::<Rootable![NamedIdUserData<'_, I>]>()
                        .unwrap()
                        .id
                        .index() as i64,
                )
            }
        }

        #[derive(Collect)]
        #[collect(no_drop, bound = "")]
        struct MethodsSingleton<'gc, I>(Gc<'gc, dyn vm::UserDataMethods<'gc>>, PhantomData<I>);

        impl<'gc, I> vm::Singleton<'gc> for MethodsSingleton<'gc, I>
        where
            I: typed_id_map::Id + 'static,
        {
            fn create(ctx: vm::Context<'gc>) -> Self {
                let methods = ctx.alloc_static(Methods::<I>(PhantomData));
                MethodsSingleton(
                    gc_arena::unsize!(methods => dyn vm::UserDataMethods<'gc>),
                    PhantomData,
                )
            }
        }

        let methods = ctx.singleton::<Rootable![MethodsSingleton<'_, I>]>().0;

        let userdata = vm::UserData::new::<Rootable![NamedIdUserData<'_, I>]>(
            &ctx,
            NamedIdUserData { id, name },
        );
        userdata.set_methods(&ctx, Some(methods));

        userdata
    }

    pub fn downcast(userdata: vm::UserData<'gc>) -> Result<&'gc Self, vm::BadUserDataType> {
        userdata.downcast::<Rootable![NamedIdUserData<'_, I>]>()
    }
}
