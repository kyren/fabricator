use std::rc::Rc;

use fabricator_vm as vm;

use crate::{
    api::{
        id_user_data::NamedIdUserData,
        magic::{DuplicateMagicName, MagicExt as _},
    },
    state::{
        Configuration, State,
        configuration::{Font, FontId},
    },
};

pub type FontUserData<'gc> = NamedIdUserData<'gc, FontId>;

pub fn font_api<'gc>(
    ctx: vm::Context<'gc>,
    config: &Configuration,
) -> Result<vm::MagicSet<'gc>, DuplicateMagicName> {
    let mut magic = vm::MagicSet::new();

    for font in config.fonts.values() {
        magic.add_constant(ctx, ctx.intern(&font.name), ctx.fetch(&font.userdata))?;
    }

    let font_get_name = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let font: vm::UserData = exec.stack().consume(ctx)?;
        let font_id = FontUserData::downcast(font)?.id;
        State::ctx_with_mut(ctx, |state| {
            exec.stack()
                .replace(ctx, ctx.intern(&state.config.fonts[font_id].name));
            Ok(())
        })?
    });
    magic
        .add_constant(ctx, ctx.intern("font_get_name"), font_get_name)
        .unwrap();

    let font_add_sprite_ext = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        State::ctx_with_mut(ctx, |state| {
            let id = state.config.fonts.insert_with_id(|id| {
                let name = format!("dynamic_font_{}", id.index());
                let vm_name = ctx.intern(&name);
                Rc::new(Font {
                    name: format!("dynamic_font_{}", id.index()),
                    userdata: ctx.stash(FontUserData::new(ctx, id, vm_name)),
                })
            });
            exec.stack()
                .replace(ctx, ctx.fetch(&state.config.fonts[id].userdata));
            Ok(())
        })?
    });
    magic
        .add_constant(ctx, ctx.intern("font_add_sprite_ext"), font_add_sprite_ext)
        .unwrap();

    Ok(magic)
}
