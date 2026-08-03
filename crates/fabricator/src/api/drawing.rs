use fabricator_math::Vec2;
use fabricator_vm as vm;

use crate::{
    api::{
        id_user_data::{IdUserData, NamedIdUserData},
        magic::{DuplicateMagicName, MagicExt as _},
    },
    state::{
        Configuration, DrawingState, DrawnSprite, DrawnSpriteFrame, EventState, SpriteId, State,
        TexturePageId,
        configuration::{ShaderId, TileSetId},
    },
};

pub type SpriteUserData<'gc> = NamedIdUserData<'gc, SpriteId>;
pub type ShaderUserData<'gc> = NamedIdUserData<'gc, ShaderId>;
pub type TileSetUserData<'gc> = NamedIdUserData<'gc, TileSetId>;
pub type TexturePageUserData = IdUserData<TexturePageId>;

pub fn drawing_api<'gc>(
    ctx: vm::Context<'gc>,
    config: &Configuration,
) -> Result<vm::MagicSet<'gc>, DuplicateMagicName> {
    let mut magic = vm::MagicSet::new();

    for sprite in config.sprites.values() {
        magic.add_constant(ctx, ctx.intern(&sprite.name), ctx.fetch(&sprite.userdata))?;
    }

    for shader in config.shaders.values() {
        magic.add_constant(ctx, ctx.intern(&shader.name), ctx.fetch(&shader.userdata))?;
    }

    for tile_set in config.tile_sets.values() {
        magic.add_constant(
            ctx,
            ctx.intern(&tile_set.name),
            ctx.fetch(&tile_set.userdata),
        )?;
    }

    magic.add_constant(ctx, ctx.intern("c_white"), 0xffffff)?;
    magic.add_constant(ctx, ctx.intern("c_black"), 0x0)?;

    let make_color_rgb = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let (r, g, b): (u8, u8, u8) = exec.stack().consume(ctx)?;
        let color = (r as u32) | (g as u32) << 8 | (b as u32) << 16;
        exec.stack().replace(ctx, color);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("make_color_rgb"), make_color_rgb)?;

    let draw_sprite = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let (sprite, sub_img, x, y): (vm::UserData, i64, f64, f64) = exec.stack().consume(ctx)?;
        let sprite = SpriteUserData::downcast(sprite)?;

        let instance = EventState::ctx_with(ctx, |e| e.instance_id)?;

        DrawingState::ctx_with_mut(ctx, |drawing_state| {
            drawing_state.drawn_sprites.push(DrawnSprite {
                instance,
                sprite: sprite.id,
                sub_img: if sub_img < 0 {
                    DrawnSpriteFrame::CurrentAnimation
                } else {
                    DrawnSpriteFrame::Frame(sub_img as usize)
                },
                position: Vec2::new(x, y),
            })
        })?;

        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("draw_sprite"), draw_sprite)?;

    let sprite_get_info = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let sprite: vm::UserData = exec.stack().consume(ctx)?;
        let sprite_id = SpriteUserData::downcast(sprite)?.id;

        let info = vm::Object::new(&ctx);
        State::ctx_with(ctx, |state| {
            let sprite = &state.config.sprites[sprite_id];
            info.set(&ctx, ctx.intern("width"), sprite.size[0] as i64);
            info.set(&ctx, ctx.intern("height"), sprite.size[1] as i64);
            info.set(&ctx, ctx.intern("xoffset"), sprite.origin[0] as i64);
            info.set(&ctx, ctx.intern("yoffset"), sprite.origin[1] as i64);

            let frame_info = vm::Array::new(&ctx);
            let frames = vm::Array::new(&ctx);
            for (i, frame) in sprite.frames.iter().enumerate() {
                let frame_info_obj = vm::Object::new(&ctx);

                let tick_rate = state.config.tick_rate;
                let playback_speed = sprite.playback_speed;
                let playback_length = sprite.playback_length;

                let next_frame_start = if i < sprite.frames.len() {
                    sprite.frames[i].frame_start
                } else {
                    playback_length
                };

                frame_info_obj.set(
                    &ctx,
                    ctx.intern("frame"),
                    frame.frame_start / playback_speed * tick_rate,
                );
                frame_info_obj.set(
                    &ctx,
                    ctx.intern("duration"),
                    (next_frame_start - frame.frame_start) / playback_speed * tick_rate,
                );
                frame_info.push(&ctx, frame_info_obj);

                let frame_obj = vm::Object::new(&ctx);
                let texture = &state.config.textures[frame.texture];
                let texture_page_id = state.config.texture_page_for_texture[frame.texture];
                let texture_page = &state.config.texture_pages[texture_page_id];
                let page_position = texture_page.textures[frame.texture];
                frame_obj.set(&ctx, ctx.intern("x"), page_position[0] as i64);
                frame_obj.set(&ctx, ctx.intern("y"), page_position[1] as i64);
                frame_obj.set(&ctx, ctx.intern("w"), texture.size[0] as i64);
                frame_obj.set(&ctx, ctx.intern("h"), texture.size[1] as i64);
                frame_obj.set(
                    &ctx,
                    ctx.intern("texture"),
                    ctx.fetch(&state.config.texture_pages[texture_page_id].userdata),
                );
                frame_obj.set(
                    &ctx,
                    ctx.intern("crop_width"),
                    texture.cropped_size[0] as i64,
                );
                frame_obj.set(
                    &ctx,
                    ctx.intern("crop_height"),
                    texture.cropped_size[1] as i64,
                );
                frame_obj.set(
                    &ctx,
                    ctx.intern("x_offset"),
                    texture.cropped_offset[0] as i64,
                );
                frame_obj.set(
                    &ctx,
                    ctx.intern("y_offset"),
                    texture.cropped_offset[1] as i64,
                );
                frames.push(&ctx, frame_obj);
            }

            info.set(&ctx, ctx.intern("frame_info"), frame_info);
            info.set(&ctx, ctx.intern("frames"), frames);
        })?;

        exec.stack().replace(ctx, info);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("sprite_get_info"), sprite_get_info)?;

    let sprite_get_name = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let sprite: vm::UserData = exec.stack().consume(ctx)?;
        let sprite_id = SpriteUserData::downcast(sprite)?.id;
        let name = State::ctx_with(ctx, |state| {
            ctx.intern(&state.config.sprites[sprite_id].name)
        })?;
        exec.stack().replace(ctx, name);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("sprite_get_name"), sprite_get_name)?;

    let sprite_get_number = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let sprite: vm::UserData = exec.stack().consume(ctx)?;
        let sprite_id = SpriteUserData::downcast(sprite)?.id;
        let frame_count =
            State::ctx_with(ctx, |state| state.config.sprites[sprite_id].frames.len())?;
        exec.stack().replace(ctx, frame_count as isize);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("sprite_get_number"), sprite_get_number)?;

    let sprite_get_width = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let sprite: vm::UserData = exec.stack().consume(ctx)?;
        let sprite_id = SpriteUserData::downcast(sprite)?.id;
        let width = State::ctx_with(ctx, |state| state.config.sprites[sprite_id].size[0])?;
        exec.stack().replace(ctx, width);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("sprite_get_width"), sprite_get_width)?;

    let sprite_get_height = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let sprite: vm::UserData = exec.stack().consume(ctx)?;
        let sprite_id = SpriteUserData::downcast(sprite)?.id;
        let height = State::ctx_with(ctx, |state| state.config.sprites[sprite_id].size[0])?;
        exec.stack().replace(ctx, height);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("sprite_get_height"), sprite_get_height)?;

    let sprite_get_xoffset = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let sprite: vm::UserData = exec.stack().consume(ctx)?;
        let sprite_id = SpriteUserData::downcast(sprite)?.id;
        let xoffset = State::ctx_with(ctx, |state| state.config.sprites[sprite_id].origin[0])?;
        exec.stack().replace(ctx, xoffset);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("sprite_get_xoffset"), sprite_get_xoffset)?;

    let sprite_get_yoffset = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let sprite: vm::UserData = exec.stack().consume(ctx)?;
        let sprite_id = SpriteUserData::downcast(sprite)?.id;
        let yoffset = State::ctx_with(ctx, |state| state.config.sprites[sprite_id].origin[1])?;
        exec.stack().replace(ctx, yoffset);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("sprite_get_yoffset"), sprite_get_yoffset)?;

    let sprite_get_texture = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let (sprite, index): (vm::UserData, usize) = exec.stack().consume(ctx)?;
        let sprite_id = SpriteUserData::downcast(sprite)?.id;
        let texture = State::ctx_with(ctx, |state| {
            let texture_id = state.config.sprites[sprite_id].frames[index].texture;
            let texture_page_id = state.config.texture_page_for_texture[texture_id];
            ctx.fetch(&state.config.texture_pages[texture_page_id].userdata)
        })?;
        exec.stack().replace(ctx, texture);
        Ok(())
    });
    magic.add_constant(ctx, ctx.intern("sprite_get_texture"), sprite_get_texture)?;

    let texture_get_texel_width = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let texture_page: vm::UserData = exec.stack().consume(ctx)?;
        let texture_page_id = TexturePageUserData::downcast(texture_page)?.id;
        State::ctx_with(ctx, |state| {
            exec.stack()
                .replace(ctx, state.config.texture_pages[texture_page_id].size[0]);
        })?;
        Ok(())
    });
    magic.add_constant(
        ctx,
        ctx.intern("texture_get_texel_width"),
        texture_get_texel_width,
    )?;

    let texture_get_texel_height = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let texture_page: vm::UserData = exec.stack().consume(ctx)?;
        let texture_page_id = TexturePageUserData::downcast(texture_page)?.id;
        State::ctx_with(ctx, |state| {
            exec.stack()
                .replace(ctx, state.config.texture_pages[texture_page_id].size[1]);
        })?;
        Ok(())
    });
    magic.add_constant(
        ctx,
        ctx.intern("texture_get_texel_height"),
        texture_get_texel_height,
    )?;

    Ok(magic)
}
