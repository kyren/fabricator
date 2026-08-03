use fabricator_vm as vm;

use crate::{
    api::{
        id_user_data::NamedIdUserData,
        magic::{DuplicateMagicName, MagicExt as _, create_magic_ro},
    },
    state::{Configuration, RoomId, State},
};

pub type RoomUserData<'gc> = NamedIdUserData<'gc, RoomId>;

pub fn room_api<'gc>(
    ctx: vm::Context<'gc>,
    config: &Configuration,
) -> Result<vm::MagicSet<'gc>, DuplicateMagicName> {
    let mut magic = vm::MagicSet::new();

    for room in config.rooms.values() {
        let room_id_ud = ctx.fetch(&room.userdata);
        let room_name = RoomUserData::downcast(room_id_ud).unwrap().name;
        magic.add_constant(ctx, room_name, room_id_ud)?;
    }

    let room_magic = create_magic_ro(ctx, |ctx| {
        Ok(State::ctx_with(ctx, |state| {
            ctx.fetch(&state.config.rooms[state.current_room.unwrap()].userdata)
                .into()
        })?)
    });
    magic.add(ctx.intern("room"), room_magic)?;

    let room_goto = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let room: vm::Value = exec.stack().consume(ctx)?;
        let room_id = match room {
            vm::Value::UserData(userdata) => RoomUserData::downcast(userdata)?.id,
            vm::Value::String(room_name) => State::ctx_with(ctx, |state| {
                match state.config.room_dict.get(room_name.as_str()) {
                    Some(&room_id) => Ok(room_id),
                    None => Err(vm::RuntimeError::msg(format!(
                        "no room named {:?}",
                        room_name.as_str()
                    ))),
                }
            })??,
            _ => {
                return Err(vm::RuntimeError::msg("cannot set room to non-room value"));
            }
        };
        State::ctx_with_mut(ctx, |state| {
            state.next_room = Some(room_id);
        })?;
        Ok(())
    });
    magic
        .add_constant(ctx, ctx.intern("room_goto"), room_goto)
        .unwrap();

    let room_get_name = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let room: vm::UserData = exec.stack().consume(ctx)?;
        let room_id = RoomUserData::downcast(room)?.id;
        State::ctx_with_mut(ctx, |state| {
            exec.stack()
                .replace(ctx, ctx.intern(&state.config.rooms[room_id].name));
            Ok(())
        })?
    });
    magic
        .add_constant(ctx, ctx.intern("room_get_name"), room_get_name)
        .unwrap();

    let room_width = create_magic_ro(ctx, |ctx| {
        Ok(State::ctx_with(ctx, |state| {
            vm::Value::Integer(state.config.rooms[state.current_room.unwrap()].size[0] as i64)
        })?)
    });
    magic.add(ctx.intern("room_width"), room_width)?;

    let room_height = create_magic_ro(ctx, |ctx| {
        Ok(State::ctx_with(ctx, |state| {
            vm::Value::Integer(state.config.rooms[state.current_room.unwrap()].size[1] as i64)
        })?)
    });
    magic.add(ctx.intern("room_height"), room_height)?;

    magic.add_constant(
        ctx,
        ctx.intern("room_first"),
        ctx.fetch(&config.rooms[config.first_room].userdata),
    )?;

    magic.add_constant(
        ctx,
        ctx.intern("room_last"),
        ctx.fetch(&config.rooms[config.last_room].userdata),
    )?;

    Ok(magic)
}
