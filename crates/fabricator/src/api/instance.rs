use std::f64;

use fabricator_vm as vm;
use gc_arena::{Collect, Gc, Rootable};

use crate::{
    api::{layer::find_layer, magic::MagicExt as _},
    project::ObjectEvent,
    state::{EventState, InstanceId, State},
};

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct InstanceUserData<'gc> {
    #[collect(require_static)]
    pub id: InstanceId,
    pub name: vm::String<'gc>,
}

impl<'gc> InstanceUserData<'gc> {
    pub fn new(ctx: vm::Context<'gc>, id: InstanceId) -> vm::UserData<'gc> {
        let methods = ctx.singleton::<Rootable![InstanceMethodsSingleton<'_>]>().0;
        let ud = vm::UserData::new::<Rootable![InstanceUserData<'_>]>(
            &ctx,
            InstanceUserData {
                id,
                name: ctx.intern(&format!("instance {}:{}", id.index(), id.generation())),
            },
        );
        ud.set_methods(&ctx, Some(methods));
        ud
    }

    pub fn downcast(userdata: vm::UserData<'gc>) -> Result<&'gc Self, vm::BadUserDataType> {
        userdata.downcast::<Rootable![InstanceUserData<'_>]>()
    }
}

#[derive(Collect)]
#[collect(no_drop)]
struct InstanceMethodsSingleton<'gc>(Gc<'gc, dyn vm::UserDataMethods<'gc>>);

impl<'gc> vm::Singleton<'gc> for InstanceMethodsSingleton<'gc> {
    fn create(ctx: vm::Context<'gc>) -> Self {
        #[derive(Collect)]
        #[collect(require_static)]
        struct Methods;

        impl Methods {
            fn do_get_field<'gc>(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                key: vm::String<'gc>,
            ) -> Result<Option<vm::Value<'gc>>, vm::RuntimeError> {
                let instance = InstanceUserData::downcast(ud).unwrap();
                State::ctx_with(ctx, |state| {
                    let instance = state
                        .instances
                        .get(instance.id)
                        .ok_or_else(|| vm::RuntimeError::msg("expired instance"))?;

                    Ok(match key.as_str() {
                        "id" => Some(ud.into()),
                        "object_index" => Some(
                            ctx.fetch(&state.config.objects[instance.object].userdata)
                                .into(),
                        ),
                        "x" => Some(instance.position[0].into()),
                        "y" => Some(instance.position[1].into()),
                        "image_angle" => Some(instance.rotation.to_degrees().into()),
                        _ => ctx.fetch(&instance.properties).get(key),
                    })
                })?
            }
        }

        impl<'gc> vm::UserDataMethods<'gc> for Methods {
            fn get_field(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                key: vm::String<'gc>,
            ) -> Result<vm::Value<'gc>, vm::RuntimeError> {
                self.do_get_field(ud, ctx, key)?
                    .ok_or_else(|| vm::RuntimeError::msg(format!("missing field {key:?}")))
            }

            fn set_field(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                key: vm::String<'gc>,
                value: vm::Value<'gc>,
            ) -> Result<(), vm::RuntimeError> {
                let instance = InstanceUserData::downcast(ud).unwrap();
                State::ctx_with_mut(ctx, |state| {
                    let instance = state
                        .instances
                        .get_mut(instance.id)
                        .ok_or_else(|| vm::RuntimeError::msg("expired instance"))?;

                    match key.as_str() {
                        "id" => Err(vm::RuntimeError::msg(format!("`id` is read-only"))),
                        "object_index" => Err(vm::RuntimeError::msg(format!(
                            "`object_index` is read-only"
                        ))),
                        "x" => {
                            instance.position[0] = vm::FromValue::from_value(ctx, value)?;
                            Ok(())
                        }
                        "y" => {
                            instance.position[1] = vm::FromValue::from_value(ctx, value)?;
                            Ok(())
                        }
                        "image_angle" => {
                            let angle_deg: f64 = vm::FromValue::from_value(ctx, value)?;
                            instance.rotation = -angle_deg.to_radians() % (f64::consts::PI * 2.0);
                            Ok(())
                        }
                        _ => {
                            ctx.fetch(&instance.properties).set(&ctx, key, value);
                            Ok(())
                        }
                    }
                })?
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

                Ok(self.do_get_field(ud, ctx, key)?.unwrap_or_default())
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

            fn coerce_string(
                &self,
                ud: vm::UserData<'gc>,
                _ctx: vm::Context<'gc>,
            ) -> Option<vm::String<'gc>> {
                Some(InstanceUserData::downcast(ud).unwrap().name)
            }
        }

        let methods = Gc::new(&ctx, Methods);
        Self(gc_arena::unsize!(methods => dyn vm::UserDataMethods<'gc>))
    }
}

pub fn instance_api<'gc>(ctx: vm::Context<'gc>) -> vm::MagicSet<'gc> {
    let mut magic = vm::MagicSet::new();

    #[derive(Debug, Copy, Clone)]
    enum EventType {
        Create,
        Destroy,
        CleanUp,
        Step,
        Other,
    }

    #[derive(Debug, Copy, Clone)]
    enum StepEvent {
        Normal,
        Begin,
        End,
    }

    #[derive(Debug, Copy, Clone)]
    enum OtherEvent {
        RoomStart,
        RoomEnd,
    }

    for (event_type, name) in [
        (EventType::Create, "ev_create"),
        (EventType::Destroy, "ev_destroy"),
        (EventType::CleanUp, "ev_cleanup"),
        (EventType::Step, "ev_step"),
        (EventType::Other, "ev_other"),
    ] {
        magic
            .add_constant(
                &ctx,
                ctx.intern(name),
                vm::UserData::new_static(&ctx, event_type),
            )
            .unwrap();
    }

    for (step_event, name) in [
        (StepEvent::Normal, "ev_step_normal"),
        (StepEvent::Begin, "ev_step_begin"),
        (StepEvent::End, "ev_step_end"),
    ] {
        magic
            .add_constant(
                &ctx,
                ctx.intern(name),
                vm::UserData::new_static(&ctx, step_event),
            )
            .unwrap();
    }

    for (other_event, name) in [
        (OtherEvent::RoomStart, "ev_room_start"),
        (OtherEvent::RoomEnd, "ev_room_end"),
    ] {
        magic
            .add_constant(
                &ctx,
                ctx.intern(name),
                vm::UserData::new_static(&ctx, other_event),
            )
            .unwrap();
    }

    let event_perform = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let (event_type, sub_event): (vm::UserData, Option<vm::UserData>) =
            exec.stack().consume(ctx)?;

        let mut instance_ud_id = None;
        for i in 0..exec.this_depth() {
            if let vm::Value::UserData(ud) = exec.this(ctx, i) {
                if let Ok(iud) = InstanceUserData::downcast(ud) {
                    instance_ud_id = Some((ud, iud.id));
                    break;
                }
            }
        }

        let Some((instance_ud, instance_id)) = instance_ud_id else {
            return Err(vm::RuntimeError::msg(
                "no instance userdata is currently on the `self` stack",
            ));
        };

        let event = match *event_type.downcast_static::<EventType>()? {
            EventType::Create => ObjectEvent::Create,
            EventType::Destroy => ObjectEvent::Destroy,
            EventType::CleanUp => ObjectEvent::CleanUp,
            EventType::Step => {
                match *sub_event
                    .ok_or_else(|| vm::RuntimeError::msg("expected sub-event for `ev_step`"))?
                    .downcast_static::<StepEvent>()?
                {
                    StepEvent::Normal => ObjectEvent::Step,
                    StepEvent::Begin => ObjectEvent::BeginStep,
                    StepEvent::End => ObjectEvent::EndStep,
                }
            }
            EventType::Other => {
                match *sub_event
                    .ok_or_else(|| vm::RuntimeError::msg("expected sub-event for `ev_other`"))?
                    .downcast_static::<OtherEvent>()?
                {
                    OtherEvent::RoomStart => ObjectEvent::RoomStart,
                    OtherEvent::RoomEnd => ObjectEvent::RoomEnd,
                }
            }
        };

        if let Some(closure) = State::ctx_with(ctx, |state| {
            state.instances[instance_id]
                .event_closures
                .get(&event)
                .cloned()
        })? {
            exec.with_this(instance_ud).call(ctx, ctx.fetch(&closure))?;
            exec.stack().clear();
        }

        Ok(())
    });
    magic
        .add_constant(&ctx, ctx.intern("event_perform"), event_perform)
        .unwrap();

    let event_inherited = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        exec.stack().clear();

        let (instance_id, object_id, current_event) = EventState::ctx_with(ctx, |state| {
            (state.instance_id, state.object_id, state.current_event)
        })?;

        if let Some((parent_object_id, closure)) = State::ctx_with(ctx, |state| {
            let parent = state.config.objects[object_id].parent?;
            Some((
                parent,
                state
                    .scripts
                    .object_events
                    .get(&parent)?
                    .get(&current_event)?
                    .clone(),
            ))
        })? {
            EventState::ctx_cell(ctx).freeze(
                &EventState {
                    instance_id,
                    object_id: parent_object_id,
                    current_event,
                },
                || {
                    exec.call(ctx, ctx.fetch(&closure))?;
                    exec.stack().clear();
                    Ok(())
                },
            )
        } else {
            Ok(())
        }
    });
    magic
        .add_constant(&ctx, ctx.intern("event_inherited"), event_inherited)
        .unwrap();

    let instance_deactivate_layer = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let layer_id_or_name: vm::Value = exec.stack().consume(ctx)?;
        let to_deactivate = State::ctx_with(ctx, |state| -> Result<_, vm::RuntimeError> {
            let layer_id = find_layer(state, layer_id_or_name)?;
            Ok(state
                .instances_for_layer
                .get(layer_id)
                .into_iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>())
        })??;

        for instance_id in to_deactivate {
            State::ctx_with_mut(ctx, |state| {
                let instance = &mut state.instances[instance_id];
                instance.active = false;
            })?;
        }

        Ok(())
    });
    magic
        .add_constant(
            &ctx,
            ctx.intern("instance_deactivate_layer"),
            instance_deactivate_layer,
        )
        .unwrap();

    magic
}
