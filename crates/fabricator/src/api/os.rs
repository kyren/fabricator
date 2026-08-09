use std::{env, fs};

use fabricator_stdlib::buffer;
use fabricator_vm as vm;

use crate::{
    api::magic::{MagicExt as _, create_magic_ro},
    state::State,
};

pub fn os_api<'gc>(ctx: vm::Context<'gc>) -> vm::MagicSet<'gc> {
    let mut magic = vm::MagicSet::new();

    magic
        .add_constant(ctx, ctx.intern("os_type"), ctx.intern(env::consts::OS))
        .unwrap();
    magic
        .add_constant(ctx, ctx.intern("os_windows"), ctx.intern("windows"))
        .unwrap();
    magic
        .add_constant(ctx, ctx.intern("os_macosx"), ctx.intern("macos"))
        .unwrap();
    magic
        .add_constant(ctx, ctx.intern("os_linux"), ctx.intern("linux"))
        .unwrap();
    magic
        .add_constant(ctx, ctx.intern("os_switch"), ctx.intern("switch"))
        .unwrap();
    magic
        .add_constant(ctx, ctx.intern("os_ps4"), ctx.intern("ps4"))
        .unwrap();
    magic
        .add_constant(ctx, ctx.intern("os_ps5"), ctx.intern("ps5"))
        .unwrap();
    magic
        .add_constant(ctx, ctx.intern("os_gdk"), ctx.intern("gdk"))
        .unwrap();
    magic
        .add_constant(
            ctx,
            ctx.intern("os_xboxseriesxs"),
            ctx.intern("xboxseriesx"),
        )
        .unwrap();

    let environment_get_variable = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let var_name: vm::String = exec.stack().consume(ctx)?;
        let env_var = match env::var(var_name.as_str()) {
            Ok(val) => ctx.intern(&val),
            Err(_) => ctx.intern(""),
        };
        exec.stack().replace(ctx, env_var);
        Ok(())
    });
    magic
        .add_constant(
            ctx,
            ctx.intern("environment_get_variable"),
            environment_get_variable,
        )
        .unwrap();

    let file_exists = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        State::ctx_with(ctx, |state| {
            let file_name: vm::String = exec.stack().consume(ctx)?;
            let path = state.config.data_path.join(file_name.as_str());
            exec.stack().replace(ctx, fs::exists(&path)?);
            Ok(())
        })?
    });
    magic
        .add_constant(ctx, ctx.intern("file_exists"), file_exists)
        .unwrap();

    let get_timer = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        State::ctx_with(ctx, |state| {
            exec.stack()
                .replace(ctx, state.start_instant.elapsed().as_micros() as i64);
            Ok(())
        })?
    });
    magic
        .add_constant(ctx, ctx.intern("get_timer"), get_timer)
        .unwrap();

    magic
        .add(
            ctx.intern("current_time"),
            create_magic_ro(ctx, |ctx| {
                State::ctx_with(ctx, |state| {
                    Ok((state.start_instant.elapsed().as_millis() as i64).into())
                })?
            }),
        )
        .unwrap();

    let show_error = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let arg: vm::String = exec.stack().consume(ctx)?;
        Err(vm::RuntimeError::msg(arg.as_str().to_owned()).into())
    });
    magic
        .add_constant(ctx, ctx.intern("show_error"), show_error)
        .unwrap();

    let buffer_load = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        State::ctx_with(ctx, |state| {
            let file_name: vm::String = exec.stack().consume(ctx)?;
            let path = state.config.data_path.join(file_name.as_str());
            let data = fs::read(path)?;
            let buffer = buffer::Buffer::new(data, buffer::BufferType::Growable, 1);
            exec.stack().replace(
                ctx,
                vm::UserData::new_static::<buffer::Buffer>(&ctx, buffer),
            );
            Ok(())
        })?
    });
    magic
        .add_constant(ctx, ctx.intern("buffer_load"), buffer_load)
        .unwrap();

    magic
}
