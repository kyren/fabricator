use fabricator_vm::{self as vm};

use crate::{
    api::magic::{MagicExt as _, create_magic_ro},
    state::{InputState, MouseButtons},
};

pub fn platform_api<'gc>(ctx: vm::Context<'gc>) -> vm::MagicSet<'gc> {
    let mut magic = vm::MagicSet::new();

    let mouse_x_magic = create_magic_ro(ctx, |ctx| {
        InputState::ctx_with(ctx, |input| Ok((input.mouse_position[0] as f64).into()))?
    });
    magic
        .add(ctx.intern_static("mouse_x"), mouse_x_magic)
        .unwrap();

    let mouse_y_magic = create_magic_ro(ctx, |ctx| {
        InputState::ctx_with(ctx, |input| Ok((input.mouse_position[1] as f64).into()))?
    });
    magic
        .add(ctx.intern_static("mouse_y"), mouse_y_magic)
        .unwrap();

    for (name, bits) in [
        ("mb_left", MouseButtons::Left.bits()),
        ("mb_middle", MouseButtons::Middle.bits()),
        ("mb_right", MouseButtons::Right.bits()),
        ("mb_any", MouseButtons::all().bits()),
    ] {
        magic
            .add_constant(ctx, ctx.intern_static(name), bits as i64)
            .unwrap();
    }

    let mouse_check_button = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let button = MouseButtons::from_bits_truncate(exec.stack().consume(ctx)?);
        InputState::ctx_with(ctx, |input| {
            exec.stack()
                .replace(ctx, !(input.mouse_pressed & button).is_empty());
            Ok(())
        })?
    });
    magic
        .add_constant(
            ctx,
            ctx.intern_static("mouse_check_button"),
            mouse_check_button,
        )
        .unwrap();

    magic
}
