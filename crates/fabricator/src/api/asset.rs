use std::collections::{HashMap, HashSet};

use fabricator_vm as vm;
use gc_arena::Gc;

use crate::{
    api::{
        drawing::{ShaderUserData, SpriteUserData, TileSetUserData},
        font::FontUserData,
        magic::{DuplicateMagicName, MagicExt as _},
        object::ObjectUserData,
        room::RoomUserData,
        sound::SoundUserData,
    },
    state::{Configuration, State},
};

#[derive(Debug, Copy, Clone)]
pub enum AssetType {
    Object,
    Sprite,
    Room,
    Font,
    Shader,
    Sound,
    TileSet,
}

pub fn assets_api<'gc>(
    ctx: vm::Context<'gc>,
    config: &Configuration,
) -> Result<vm::MagicSet<'gc>, DuplicateMagicName> {
    let mut assets_map = HashMap::new();

    for sprite in config.sprites.values() {
        assets_map.insert(ctx.intern(&sprite.name), ctx.fetch(&sprite.userdata));
    }

    for font in config.fonts.values() {
        assets_map.insert(ctx.intern(&font.name), ctx.fetch(&font.userdata));
    }

    for sound in config.sounds.values() {
        assets_map.insert(ctx.intern(&sound.name), ctx.fetch(&sound.userdata));
    }

    for tile_set in config.tile_sets.values() {
        assets_map.insert(ctx.intern(&tile_set.name), ctx.fetch(&tile_set.userdata));
    }

    for shader in config.shaders.values() {
        assets_map.insert(ctx.intern(&shader.name), ctx.fetch(&shader.userdata));
    }

    for room in config.rooms.values() {
        assets_map.insert(ctx.intern(&room.name), ctx.fetch(&room.userdata));
    }

    for object in config.objects.values() {
        assets_map.insert(ctx.intern(&object.name), ctx.fetch(&object.userdata));
    }

    let assets_map = Gc::new(&ctx, assets_map);

    let mut magic = vm::MagicSet::new();

    for (asset_type, type_name) in [
        (AssetType::Object, "asset_object"),
        (AssetType::Sprite, "asset_sprite"),
        (AssetType::Room, "asset_room"),
        (AssetType::Font, "asset_font"),
        (AssetType::Shader, "asset_shader"),
        (AssetType::Sound, "asset_sound"),
        (AssetType::TileSet, "asset_tiles"),
    ] {
        magic.add_constant(
            &ctx,
            ctx.intern(type_name),
            vm::UserData::new_static(&ctx, asset_type),
        )?;
    }

    let asset_get_ids = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let asset_type: vm::UserData = exec.stack().consume(ctx)?;
        let asset_type = *asset_type.downcast_static::<AssetType>()?;
        let ids = vm::Array::new(&ctx);
        State::ctx_with(ctx, |state| match asset_type {
            AssetType::Object => {
                ids.extend(
                    &ctx,
                    state
                        .config
                        .objects
                        .values()
                        .map(|o| ctx.fetch(&o.userdata).into()),
                );
            }
            AssetType::Sprite => {
                ids.extend(
                    &ctx,
                    state
                        .config
                        .sprites
                        .values()
                        .map(|o| ctx.fetch(&o.userdata).into()),
                );
            }
            AssetType::Room => {
                ids.extend(
                    &ctx,
                    state
                        .config
                        .rooms
                        .values()
                        .map(|o| ctx.fetch(&o.userdata).into()),
                );
            }
            AssetType::Font => {
                ids.extend(
                    &ctx,
                    state
                        .config
                        .fonts
                        .values()
                        .map(|f| ctx.fetch(&f.userdata).into()),
                );
            }
            AssetType::Shader => {
                ids.extend(
                    &ctx,
                    state
                        .config
                        .shaders
                        .values()
                        .map(|s| ctx.fetch(&s.userdata).into()),
                );
            }
            AssetType::Sound => {
                ids.extend(
                    &ctx,
                    state
                        .config
                        .sounds
                        .values()
                        .map(|s| ctx.fetch(&s.userdata).into()),
                );
            }
            AssetType::TileSet => {
                ids.extend(
                    &ctx,
                    state
                        .config
                        .tile_sets
                        .values()
                        .map(|t| ctx.fetch(&t.userdata).into()),
                );
            }
        })?;
        exec.stack().replace(ctx, ids);
        Ok(())
    });
    magic.add_constant(&ctx, ctx.intern("asset_get_ids"), asset_get_ids)?;

    let asset_get_index =
        vm::Callback::from_fn_with_root(&ctx, assets_map, |&assets_map, ctx, mut exec| {
            let name: vm::String = exec.stack().consume(ctx)?;
            let asset = assets_map
                .get(&name)
                .copied()
                .map(|v| v.into())
                .unwrap_or(vm::Value::Undefined);
            exec.stack().replace(ctx, asset);
            Ok(())
        });
    magic.add_constant(&ctx, ctx.intern("asset_get_index"), asset_get_index)?;

    let asset_has_tags =
        vm::Callback::from_fn_with_root(&ctx, assets_map, |&assets_map, ctx, mut exec| {
            let (name_or_id, tag_or_tags): (vm::Value, vm::Value) = exec.stack().consume(ctx)?;
            let id =
                if let vm::Value::UserData(ud) = name_or_id {
                    ud
                } else {
                    let name = name_or_id.as_string().ok_or_else(|| vm::RuntimeError::msg(format!(
                    "`asset_has_tags` expects asset handle or name as first argument, got {}",
                    name_or_id.type_name()
                )))?;
                    assets_map.get(&name).copied().ok_or_else(|| {
                        vm::RuntimeError::msg(format!("no such asset named {name}"))
                    })?
                };

            let has_tags = State::ctx_with(ctx, |state| {
                let empty_tags = HashSet::new();

                let tags = if let Ok(room) = RoomUserData::downcast(id) {
                    &state.config.rooms[room.id].tags
                } else if let Ok(object) = ObjectUserData::downcast(id) {
                    &state.config.objects[object.id].tags
                } else if let Ok(_) = SpriteUserData::downcast(id) {
                    &empty_tags
                } else if let Ok(_) = FontUserData::downcast(id) {
                    &empty_tags
                } else if let Ok(_) = ShaderUserData::downcast(id) {
                    &empty_tags
                } else if let Ok(_) = SoundUserData::downcast(id) {
                    &empty_tags
                } else if let Ok(_) = TileSetUserData::downcast(id) {
                    &empty_tags
                } else {
                    return Err(vm::RuntimeError::msg("userdata is not an asset id"));
                };

                match tag_or_tags {
                    vm::Value::String(s) => Ok(tags.contains(s.as_str())),
                    vm::Value::Array(array) => {
                        let mut has_tag = true;
                        for i in 0..array.len() {
                            let s = array
                                .get(i)
                                .unwrap()
                                .as_string()
                                .ok_or_else(|| vm::RuntimeError::msg("tag must be a string"))?;
                            if !tags.contains(s.as_str()) {
                                has_tag = false;
                                break;
                            }
                        }
                        Ok(has_tag)
                    }
                    _ => {
                        return Err(vm::RuntimeError::msg(
                            "tags argument must be a string or an array of strings",
                        ));
                    }
                }
            })??;

            exec.stack().replace(ctx, has_tags);
            Ok(())
        });
    magic.add_constant(&ctx, ctx.intern("asset_has_tags"), asset_has_tags)?;

    Ok(magic)
}
