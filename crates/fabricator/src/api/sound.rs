use fabricator_vm as vm;

use crate::{
    api::{
        id_user_data::NamedIdUserData,
        magic::{DuplicateMagicName, MagicExt as _},
    },
    state::{Configuration, State, configuration::SoundId},
};

pub type SoundUserData<'gc> = NamedIdUserData<'gc, SoundId>;

pub fn sound_api<'gc>(
    ctx: vm::Context<'gc>,
    config: &Configuration,
) -> Result<vm::MagicSet<'gc>, DuplicateMagicName> {
    let mut magic = vm::MagicSet::new();

    for sound in config.sounds.values() {
        magic.add_constant(ctx, ctx.intern(&sound.name), ctx.fetch(&sound.userdata))?;
    }

    let audio_sound_length = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let sound: vm::UserData = exec.stack().consume(ctx)?;
        let sound_id = SoundUserData::downcast(sound)?.id;
        let duration = State::ctx_with(ctx, |state| state.config.sounds[sound_id].duration)?;
        exec.stack().replace(ctx, duration.as_secs_f64());
        Ok(())
    });
    magic
        .add_constant(ctx, ctx.intern_static("audio_sound_length"), audio_sound_length)
        .unwrap();

    Ok(magic)
}
