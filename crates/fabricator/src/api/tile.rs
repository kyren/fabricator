use fabricator_vm as vm;

use crate::{
    api::{
        drawing::TileSetUserData, id_user_data::IdUserData, layer::find_layer, magic::MagicExt as _,
    },
    state::{State, state::TileMapId},
};

pub type TileMapUserData = IdUserData<TileMapId>;

pub fn find_tile_map<'gc>(
    state: &State,
    tile_map_id: vm::UserData<'gc>,
) -> Result<TileMapId, vm::RuntimeError> {
    let id = TileMapUserData::downcast(tile_map_id)?.id;
    if state.tile_maps.contains(id) {
        Ok(id)
    } else {
        Err(vm::RuntimeError::msg("expired tile map ID"))
    }
}

pub fn tiles_api<'gc>(ctx: vm::Context<'gc>) -> vm::MagicSet<'gc> {
    let mut magic = vm::MagicSet::new();

    magic
        .add_constant(ctx, ctx.intern("tile_index_mask"), (1 << 20) - 1)
        .unwrap();

    magic
        .add_constant(
            ctx,
            ctx.intern("layer_tilemap_get_id"),
            vm::Callback::from_fn(ctx, |ctx, mut exec| {
                let mut stack = exec.stack();
                let layer_id_or_name: vm::Value = stack.consume(ctx)?;
                State::ctx_with(ctx, |state| {
                    let layer_id = find_layer(state, layer_id_or_name)?;
                    let layer = &state.layers[layer_id];
                    if let Some(tile_map_id) = layer.tile_map {
                        stack.replace(ctx, ctx.fetch(&state.tile_maps[tile_map_id].this));
                    } else {
                        stack.replace(ctx, -1);
                    }
                    Ok(())
                })?
            }),
        )
        .unwrap();

    magic
        .add_constant(
            ctx,
            ctx.intern("tilemap_tileset"),
            vm::Callback::from_fn(ctx, |ctx, mut exec| {
                let (tile_map_ud, tile_set_ud): (vm::UserData, vm::UserData) =
                    exec.stack().consume(ctx)?;
                let tile_set_id = TileSetUserData::downcast(tile_set_ud)?.id;

                State::ctx_with_mut(ctx, |state| {
                    let tile_map_id = find_tile_map(state, tile_map_ud)?;
                    state.tile_maps[tile_map_id].tile_set = Some(tile_set_id);
                    Ok(())
                })?
            }),
        )
        .unwrap();

    magic
        .add_constant(
            ctx,
            ctx.intern("tilemap_get_x"),
            vm::Callback::from_fn(ctx, |ctx, mut exec| {
                State::ctx_with(ctx, |state| {
                    let tile_map_ud: vm::UserData = exec.stack().consume(ctx)?;
                    let tile_map_id = find_tile_map(state, tile_map_ud)?;
                    exec.stack()
                        .replace(ctx, state.tile_maps[tile_map_id].position[0]);
                    Ok(())
                })?
            }),
        )
        .unwrap();

    magic
        .add_constant(
            ctx,
            ctx.intern("tilemap_get_y"),
            vm::Callback::from_fn(ctx, |ctx, mut exec| {
                State::ctx_with(ctx, |state| {
                    let tile_map_ud: vm::UserData = exec.stack().consume(ctx)?;
                    let tile_map_id = find_tile_map(state, tile_map_ud)?;
                    exec.stack()
                        .replace(ctx, state.tile_maps[tile_map_id].position[1]);
                    Ok(())
                })?
            }),
        )
        .unwrap();

    magic
}
