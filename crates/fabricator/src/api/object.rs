use fabricator_collision::bound_box_tree::BoundBoxQuery;
use fabricator_math::{Box2, Vec2};
use fabricator_vm as vm;
use gc_arena::{Collect, Gc, Rootable};

use crate::{
    api::{
        instance::InstanceUserData,
        layer::{LayerIdUserData, find_layer},
        magic::{DuplicateMagicName, MagicExt as _},
    },
    project::ObjectEvent,
    state::{Configuration, EventState, Instance, InstanceId, ObjectId, State, state::Layer},
};

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct ObjectUserData<'gc> {
    #[collect(require_static)]
    pub id: ObjectId,
    pub name: vm::String<'gc>,
}

impl<'gc> ObjectUserData<'gc> {
    pub fn new(ctx: vm::Context<'gc>, id: ObjectId, name: vm::String<'gc>) -> vm::UserData<'gc> {
        fn singleton_instance(
            state: &State,
            object_id: ObjectId,
        ) -> Result<InstanceId, vm::RuntimeError> {
            if let Some(set) = state.instances_for_object.get(object_id) {
                if !set.is_empty() {
                    if set.len() > 1 {
                        return Err(vm::RuntimeError::msg(
                            "propery access on objects only allowed on singletons",
                        ));
                    } else {
                        return Ok(*set.iter().next().unwrap());
                    }
                }
            }
            Err(vm::RuntimeError::msg(
                "propery access on object without an instance",
            ))
        }

        #[derive(Collect)]
        #[collect(no_drop)]
        struct Methods<'gc> {
            instance_iter: vm::Callback<'gc>,
        }

        impl<'gc> vm::UserDataMethods<'gc> for Methods<'gc> {
            fn get_field(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                key: vm::String<'gc>,
            ) -> Result<vm::Value<'gc>, vm::RuntimeError> {
                let object_id = ud.downcast::<Rootable![ObjectUserData<'_>]>().unwrap().id;
                let instance_ud = State::ctx_with(ctx, |state| -> Result<_, vm::RuntimeError> {
                    let instance_id = singleton_instance(state, object_id)?;
                    Ok(ctx.fetch(&state.instances[instance_id].this))
                })??;
                instance_ud.get_field(ctx, key)
            }

            fn set_field(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                key: vm::String<'gc>,
                value: vm::Value<'gc>,
            ) -> Result<(), vm::RuntimeError> {
                let object_id = ud.downcast::<Rootable![ObjectUserData<'_>]>().unwrap().id;
                let instance_ud = State::ctx_with(ctx, |state| -> Result<_, vm::RuntimeError> {
                    let instance_id = singleton_instance(state, object_id)?;
                    Ok(ctx.fetch(&state.instances[instance_id].this))
                })??;
                instance_ud.set_field(ctx, key, value)
            }

            fn get_index(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                indexes: &[vm::Value<'gc>],
            ) -> Result<vm::Value<'gc>, vm::RuntimeError> {
                if indexes.len() != 1 {
                    return Err(vm::RuntimeError::msg("object userdata expects 1 index"));
                }
                let key = vm::FromValue::from_value(ctx, indexes[0])?;

                self.get_field(ud, ctx, key)
            }

            fn set_index(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                indexes: &[vm::Value<'gc>],
                value: vm::Value<'gc>,
            ) -> Result<(), vm::RuntimeError> {
                if indexes.len() != 1 {
                    return Err(vm::RuntimeError::msg("object userdata expects 1 index"));
                }
                let key = vm::FromValue::from_value(ctx, indexes[0])?;

                self.set_field(ud, ctx, key, value)
            }

            fn iter(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
            ) -> Result<vm::UserDataIter<'gc>, vm::RuntimeError> {
                let object_id = ud.downcast::<Rootable![ObjectUserData<'_>]>().unwrap().id;

                let array = State::ctx_with(ctx, |state| {
                    vm::Array::from_iter(
                        &ctx,
                        state
                            .instances_for_object
                            .get(object_id)
                            .into_iter()
                            .flatten()
                            .filter_map(|&instance_id| {
                                let instance = &state.instances[instance_id];
                                if instance.active {
                                    Some(ctx.fetch(&instance.this).into())
                                } else {
                                    None
                                }
                            }),
                    )
                })?;

                Ok(vm::UserDataIter::Iter {
                    iter: self.instance_iter.into(),
                    state: array.into(),
                    control: 0.into(),
                })
            }

            fn coerce_string(
                &self,
                ud: vm::UserData<'gc>,
                _ctx: vm::Context<'gc>,
            ) -> Option<vm::String<'gc>> {
                Some(ud.downcast::<Rootable![ObjectUserData<'_>]>().unwrap().name)
            }

            fn coerce_integer(&self, ud: vm::UserData<'gc>, _ctx: vm::Context<'gc>) -> Option<i64> {
                Some(
                    ud.downcast::<Rootable![ObjectUserData<'_>]>()
                        .unwrap()
                        .id
                        .index() as i64,
                )
            }
        }

        #[derive(Collect)]
        #[collect(no_drop)]
        struct MethodsSingleton<'gc>(Gc<'gc, dyn vm::UserDataMethods<'gc>>);

        impl<'gc> vm::Singleton<'gc> for MethodsSingleton<'gc> {
            fn create(ctx: vm::Context<'gc>) -> Self {
                let instance_iter = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
                    let (array, mut idx): (vm::Array, usize) = exec.stack().consume(ctx)?;
                    let next_instance =
                        State::ctx_with(ctx, |state| -> Result<_, vm::RuntimeError> {
                            while idx < array.len() {
                                let ud: vm::UserData =
                                    vm::FromValue::from_value(ctx, array.get(idx).unwrap())?;
                                let instance = InstanceUserData::downcast(ud)?;
                                if state.instances.get(instance.id).is_some_and(|i| i.active) {
                                    return Ok(Some(ud));
                                }
                                idx += 1;
                            }
                            Ok(None)
                        })??;

                    if let Some(next_instance) = next_instance {
                        exec.stack().replace(ctx, (idx as isize + 1, next_instance))
                    } else {
                        exec.stack().clear();
                    }
                    Ok(())
                });

                let methods = Gc::new(&ctx, Methods { instance_iter });
                MethodsSingleton(gc_arena::unsize!(methods => dyn vm::UserDataMethods<'gc>))
            }
        }

        let methods = ctx.singleton::<Rootable![MethodsSingleton<'_>]>().0;

        let ud =
            vm::UserData::new::<Rootable![ObjectUserData<'_>]>(&ctx, ObjectUserData { id, name });
        ud.set_methods(&ctx, Some(methods));
        ud
    }

    pub fn downcast(userdata: vm::UserData<'gc>) -> Result<&'gc Self, vm::BadUserDataType> {
        userdata.downcast::<Rootable![ObjectUserData<'_>]>()
    }
}

pub fn no_one<'gc>(ctx: vm::Context<'gc>) -> vm::UserData<'gc> {
    #[derive(Collect)]
    #[collect(require_static)]
    struct NoOne;

    #[derive(Collect)]
    #[collect(no_drop)]
    struct Singleton<'gc>(vm::UserData<'gc>);

    impl<'gc> vm::Singleton<'gc> for Singleton<'gc> {
        fn create(ctx: vm::Context<'gc>) -> Self {
            #[derive(Collect)]
            #[collect(no_drop)]
            struct Methods<'gc> {
                null_iter: vm::Callback<'gc>,
            }

            impl<'gc> vm::UserDataMethods<'gc> for Methods<'gc> {
                fn iter(
                    &self,
                    _ud: vm::UserData<'gc>,
                    _ctx: vm::Context<'gc>,
                ) -> Result<vm::UserDataIter<'gc>, vm::RuntimeError> {
                    Ok(vm::UserDataIter::Iter {
                        iter: self.null_iter.into(),
                        state: vm::Value::Undefined,
                        control: vm::Value::Undefined,
                    })
                }
            }

            let ud = vm::UserData::new_static(&ctx, NoOne);
            let null_iter = vm::Callback::from_fn(&ctx, |_, _| Ok(()));
            let methods =
                gc_arena::unsize!(Gc::new(&ctx, Methods { null_iter }) => dyn vm::UserDataMethods);
            ud.set_methods(&ctx, Some(methods));

            Singleton(ud)
        }
    }

    ctx.singleton::<Rootable![Singleton<'_>]>().0
}

pub fn all<'gc>(ctx: vm::Context<'gc>) -> vm::UserData<'gc> {
    #[derive(Collect)]
    #[collect(require_static)]
    struct All;

    #[derive(Collect)]
    #[collect(no_drop)]
    struct Singleton<'gc>(vm::UserData<'gc>);

    impl<'gc> vm::Singleton<'gc> for Singleton<'gc> {
        fn create(ctx: vm::Context<'gc>) -> Self {
            #[derive(Collect)]
            #[collect(no_drop)]
            struct Methods<'gc> {
                instance_iter: vm::Callback<'gc>,
            }

            impl<'gc> vm::UserDataMethods<'gc> for Methods<'gc> {
                fn iter(
                    &self,
                    _ud: vm::UserData<'gc>,
                    ctx: vm::Context<'gc>,
                ) -> Result<vm::UserDataIter<'gc>, vm::RuntimeError> {
                    let array = State::ctx_with(ctx, |state| {
                        vm::Array::from_iter(
                            &ctx,
                            state.instances.values().filter_map(|instance| {
                                if instance.active {
                                    Some(ctx.fetch(&instance.this).into())
                                } else {
                                    None
                                }
                            }),
                        )
                    })?;

                    Ok(vm::UserDataIter::Iter {
                        iter: self.instance_iter.into(),
                        state: array.into(),
                        control: 0.into(),
                    })
                }
            }

            let instance_iter = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
                let (array, mut idx): (vm::Array, usize) = exec.stack().consume(ctx)?;
                let next_instance =
                    State::ctx_with(ctx, |state| -> Result<_, vm::RuntimeError> {
                        while idx < array.len() {
                            let ud: vm::UserData =
                                vm::FromValue::from_value(ctx, array.get(idx).unwrap())?;
                            let instance = InstanceUserData::downcast(ud)?;
                            if state.instances.get(instance.id).is_some_and(|i| i.active) {
                                idx += 1;
                                return Ok(Some(ud));
                            }
                            idx += 1;
                        }
                        Ok(None)
                    })??;

                if let Some(next_instance) = next_instance {
                    exec.stack().replace(ctx, (idx as isize + 1, next_instance))
                } else {
                    exec.stack().clear();
                }
                Ok(())
            });

            let methods = gc_arena::unsize!(
                Gc::new(&ctx, Methods { instance_iter }) => dyn vm::UserDataMethods
            );

            let ud = vm::UserData::new_static(&ctx, All);
            ud.set_methods(&ctx, Some(methods));

            Singleton(ud)
        }
    }

    ctx.singleton::<Rootable![Singleton<'_>]>().0
}

pub fn object_api<'gc>(
    ctx: vm::Context<'gc>,
    config: &Configuration,
) -> Result<vm::MagicSet<'gc>, DuplicateMagicName> {
    let mut magic = vm::MagicSet::new();

    magic
        .add_constant(&ctx, ctx.intern("noone"), no_one(ctx))
        .unwrap();

    magic
        .add_constant(&ctx, ctx.intern("all"), all(ctx))
        .unwrap();

    for object in config.objects.values() {
        magic.add_constant(&ctx, ctx.intern(&object.name), ctx.fetch(&object.userdata))?;
    }

    let object_get_name = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let object: vm::UserData = exec.stack().consume(ctx)?;
        let object_id = ObjectUserData::downcast(object)?.id;
        State::ctx_with_mut(ctx, |state| {
            exec.stack()
                .replace(ctx, ctx.intern(&state.config.objects[object_id].name));
            Ok(())
        })?
    });
    magic
        .add_constant(&ctx, ctx.intern("object_get_name"), object_get_name)
        .unwrap();

    let instance_create_depth = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let (x, y, depth, object, set_properties): (
            f64,
            f64,
            i32,
            vm::UserData,
            Option<vm::Object>,
        ) = exec.stack().consume(ctx)?;

        let object = ObjectUserData::downcast(object)?;

        let (instance_id, instance_ud, create_script) = State::ctx_with_mut(ctx, |state| {
            let properties = vm::Object::new(&ctx);
            if let Some(set_properties) = set_properties {
                // We only copy properties from the topmost object, the documentation of GMS2 is
                // vague on this point.
                //
                // TODO: Actually check the behavior against GMS2
                let set_properties = set_properties.borrow();
                for (&key, &value) in &set_properties.map {
                    properties.set(&ctx, key, value);
                }
            }

            let layer_id = state.layers.insert_with_id(|id| {
                let layer_ud = LayerIdUserData::new(ctx, id, None);
                Layer {
                    this: ctx.stash(layer_ud),
                    depth,
                    visible: true,
                    tile_map: None,
                }
            });

            let event_closures = state.event_closures(object.id);

            let instance_id = state.instances.insert_with_id(|instance_id| Instance {
                this: ctx.stash(InstanceUserData::new(ctx, instance_id)),
                object: object.id,
                active: true,
                dead: false,
                position: Vec2::new(x, y),
                rotation: 0.0,
                layer: layer_id,
                properties: ctx.stash(properties),
                event_closures,
                animation_time: 0.0,
            });

            assert!(
                state
                    .instances_for_object
                    .get_or_insert_default(object.id)
                    .insert(instance_id)
            );
            assert!(
                state
                    .instances_for_layer
                    .get_or_insert_default(layer_id)
                    .insert(instance_id)
            );

            (
                instance_id,
                ctx.fetch(&state.instances[instance_id].this),
                state.instances[instance_id]
                    .event_closures
                    .get(&ObjectEvent::Create)
                    .cloned(),
            )
        })?;

        if let Some(create_script) = create_script {
            EventState::ctx_cell(ctx).freeze(
                &EventState {
                    instance_id,
                    object_id: object.id,
                    current_event: ObjectEvent::Create,
                },
                || {
                    exec.with_this(instance_ud)
                        .call(ctx, ctx.fetch(&create_script))
                },
            )?;
        }

        exec.stack().replace(ctx, instance_ud);

        Ok(())
    });
    magic.add_constant(
        &ctx,
        ctx.intern("instance_create_depth"),
        instance_create_depth,
    )?;

    let instance_create_layer = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let (x, y, layer_id_or_name, object, set_properties): (
            f64,
            f64,
            vm::Value,
            vm::UserData,
            Option<vm::Object>,
        ) = exec.stack().consume(ctx)?;

        let object = ObjectUserData::downcast(object)?;

        let (instance_id, instance_ud, create_script) =
            State::ctx_with_mut(ctx, |state| -> Result<_, vm::RuntimeError> {
                let layer_id = find_layer(state, layer_id_or_name)?;

                let properties = vm::Object::new(&ctx);
                if let Some(set_properties) = set_properties {
                    // We only copy properties from the topmost object, see above.
                    let set_properties = set_properties.borrow();
                    for (&key, &value) in &set_properties.map {
                        properties.set(&ctx, key, value);
                    }
                }

                let event_closures = state.event_closures(object.id);

                let instance_id = state.instances.insert_with_id(|instance_id| Instance {
                    this: ctx.stash(InstanceUserData::new(ctx, instance_id)),
                    object: object.id,
                    active: true,
                    dead: false,
                    position: Vec2::new(x, y),
                    rotation: 0.0,
                    layer: layer_id,
                    properties: ctx.stash(properties),
                    event_closures,
                    animation_time: 0.0,
                });

                assert!(
                    state
                        .instances_for_object
                        .get_or_insert_default(object.id)
                        .insert(instance_id)
                );
                assert!(
                    state
                        .instances_for_layer
                        .get_or_insert_default(layer_id)
                        .insert(instance_id)
                );

                Ok((
                    instance_id,
                    ctx.fetch(&state.instances[instance_id].this),
                    state.instances[instance_id]
                        .event_closures
                        .get(&ObjectEvent::Create)
                        .cloned(),
                ))
            })??;

        if let Some(create_script) = create_script {
            EventState::ctx_cell(ctx).freeze(
                &EventState {
                    instance_id,
                    object_id: object.id,
                    current_event: ObjectEvent::Create,
                },
                || {
                    exec.with_this(instance_ud)
                        .call(ctx, ctx.fetch(&create_script))
                },
            )?;
        }

        exec.stack().replace(ctx, instance_ud);

        Ok(())
    });
    magic.add_constant(
        &ctx,
        ctx.intern("instance_create_layer"),
        instance_create_layer,
    )?;

    let instance_exists = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let object_or_instance: vm::UserData = exec.stack().consume(ctx)?;
        let found = if let Ok(object) = ObjectUserData::downcast(object_or_instance) {
            State::ctx_with(ctx, |state| {
                if let Some(set) = state.instances_for_object.get(object.id) {
                    for &instance_id in set {
                        if state.instances[instance_id].active {
                            return true;
                        }
                    }
                }
                false
            })?
        } else if let Ok(instance) = InstanceUserData::downcast(object_or_instance) {
            State::ctx_with(ctx, |state| {
                state.instances.get(instance.id).is_some_and(|i| i.active)
            })?
        } else {
            return Err(vm::RuntimeError::msg(
                "`instance_exists` expects an object or instance",
            ));
        };
        exec.stack().replace(ctx, found);
        Ok(())
    });
    magic
        .add_constant(&ctx, ctx.intern("instance_exists"), instance_exists)
        .unwrap();

    let instance_deactivate_object = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let object_or_instance: vm::UserData = exec.stack().consume(ctx)?;
        if let Ok(object) = ObjectUserData::downcast(object_or_instance) {
            State::ctx_with_mut(ctx, |state| {
                if let Some(set) = state.instances_for_object.get(object.id) {
                    for &instance_id in set {
                        state.instances[instance_id].active = false;
                    }
                }
            })?;
        } else if let Ok(instance) = InstanceUserData::downcast(object_or_instance) {
            State::ctx_with_mut(ctx, |state| {
                if let Some(instance) = state.instances.get_mut(instance.id) {
                    instance.active = false;
                }
            })?;
        } else {
            return Err(vm::RuntimeError::msg(
                "`instance_deactivate_object` expects an object or instance",
            ));
        };
        Ok(())
    });
    magic
        .add_constant(
            &ctx,
            ctx.intern("instance_deactivate_object"),
            instance_deactivate_object,
        )
        .unwrap();

    let instance_activate_region = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let (left, top, width, height, inside): (f64, f64, f64, f64, bool) =
            exec.stack().consume(ctx)?;
        if !inside {
            return Err(vm::RuntimeError::msg(
                "outside instance activation unsupported",
            ));
        }
        State::ctx_with_mut(ctx, |state| {
            let mut query = BoundBoxQuery::default();
            for &instance_id in query.intersects(
                &state.instance_bound_tree,
                Box2::with_size(Vec2::new(left, top), Vec2::new(width, height)),
            ) {
                if let Some(instance) = state.instances.get_mut(instance_id) {
                    instance.active = true;
                }
            }
        })?;
        Ok(())
    });
    magic
        .add_constant(
            &ctx,
            ctx.intern("instance_activate_region"),
            instance_activate_region,
        )
        .unwrap();

    let instance_destroy = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let object_or_instance = if let Some(ud) = exec.stack().consume(ctx)? {
            ud
        } else {
            vm::FromValue::from_value(ctx, exec.this())?
        };

        let mut to_destroy = Vec::new();
        State::ctx_with(ctx, |state| {
            if let Ok(object) = ObjectUserData::downcast(object_or_instance) {
                if let Some(set) = state.instances_for_object.get(object.id) {
                    to_destroy.extend(
                        set.iter()
                            .copied()
                            .filter(|&id| state.instances.get(id).is_some_and(|i| !i.dead)),
                    );
                }
            } else if let Ok(instance) = InstanceUserData::downcast(object_or_instance) {
                if state.instances.get(instance.id).is_some_and(|i| !i.dead) {
                    to_destroy.push(instance.id);
                }
            } else {
                return Err(vm::RuntimeError::msg(
                    "`instance_destroy` expects an object or instance",
                ));
            };

            Ok(())
        })??;

        for instance_id in to_destroy {
            if let Some(destroy_closure) = State::ctx_with(ctx, |state| {
                state.instances[instance_id]
                    .event_closures
                    .get(&ObjectEvent::Destroy)
                    .cloned()
            })? {
                let instance_ud =
                    State::ctx_with(ctx, |state| ctx.fetch(&state.instances[instance_id].this))?;
                exec.with_this(instance_ud)
                    .call(ctx, ctx.fetch(&destroy_closure))?;
                exec.stack().clear();
            }

            if let Some(clean_up_closure) = State::ctx_with(ctx, |state| {
                state.instances[instance_id]
                    .event_closures
                    .get(&ObjectEvent::CleanUp)
                    .cloned()
            })? {
                let instance_ud =
                    State::ctx_with(ctx, |state| ctx.fetch(&state.instances[instance_id].this))?;
                exec.with_this(instance_ud)
                    .call(ctx, ctx.fetch(&clean_up_closure))?;
                exec.stack().clear();
            }

            State::ctx_with_mut(ctx, |state| {
                let instance = &mut state.instances[instance_id];
                instance.active = false;
                instance.dead = true;
            })?;
        }

        Ok(())
    });
    magic
        .add_constant(&ctx, ctx.intern("instance_destroy"), instance_destroy)
        .unwrap();

    Ok(magic)
}
