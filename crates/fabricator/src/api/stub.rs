use std::env;

use fabricator_vm as vm;

use crate::api::magic::MagicExt as _;

pub fn stub_api<'gc>(ctx: vm::Context<'gc>) -> vm::MagicSet<'gc> {
    fn create_stub_constant<'gc>(
        ctx: vm::Context<'gc>,
        magic: &mut vm::MagicSet<'gc>,
        name: &'static str,
        value: impl Into<vm::Value<'gc>>,
    ) {
        magic
            .add_constant(ctx, ctx.intern_static(name), value.into())
            .unwrap();
    }

    fn create_stub_callback<'gc, const RET_COUNT: usize>(
        ctx: vm::Context<'gc>,
        magic: &mut vm::MagicSet<'gc>,
        name: &'static str,
        returns: [vm::Value<'gc>; RET_COUNT],
    ) {
        let stub_callback =
            vm::Callback::from_fn_with_root(ctx, returns, move |returns, _ctx, mut exec| {
                log::debug!("call of stubbed out callback {name}");
                exec.stack().clear();
                exec.stack().extend(returns.iter().copied());
                Ok(())
            });
        magic
            .add_constant(ctx, ctx.intern_static(name), stub_callback)
            .unwrap();
    }

    let mut magic = vm::MagicSet::new();

    let unit_userdata: vm::Value = vm::UserData::new_static(&ctx, ()).into();

    create_stub_constant(
        ctx,
        &mut magic,
        "game_save_id",
        ctx.intern(env::temp_dir().to_str().expect("tempdir not utf-8")),
    );
    create_stub_constant(
        ctx,
        &mut magic,
        "game_project_name",
        ctx.intern_static("fabricator-project"),
    );
    create_stub_callback(ctx, &mut magic, "gc_collect", []);
    create_stub_callback(ctx, &mut magic, "gc_enable", []);
    create_stub_callback(ctx, &mut magic, "exception_unhandled_handler", []);

    create_stub_callback(ctx, &mut magic, "buffer_save", []);
    create_stub_callback(ctx, &mut magic, "file_rename", [true.into()]);
    create_stub_callback(ctx, &mut magic, "directory_exists", [true.into()]);
    create_stub_callback(ctx, &mut magic, "directory_destroy", [true.into()]);
    create_stub_callback(
        ctx,
        &mut magic,
        "file_find_first",
        [ctx.intern_static("").into()],
    );
    create_stub_callback(ctx, &mut magic, "file_find_close", []);

    create_stub_callback(ctx, &mut magic, "application_surface_enable", []);
    create_stub_callback(ctx, &mut magic, "camera_set_view_pos", []);
    create_stub_callback(ctx, &mut magic, "camera_set_view_size", []);
    create_stub_callback(ctx, &mut magic, "display_reset", []);
    create_stub_callback(ctx, &mut magic, "display_set_sleep_margin", []);
    create_stub_callback(ctx, &mut magic, "display_get_width", [1366.into()]);
    create_stub_callback(ctx, &mut magic, "display_get_height", [768.into()]);
    create_stub_callback(ctx, &mut magic, "gpu_set_zfunc", []);
    create_stub_callback(ctx, &mut magic, "gpu_set_ztestenable", []);
    create_stub_callback(ctx, &mut magic, "gpu_set_zwriteenable", []);
    create_stub_callback(ctx, &mut magic, "shader_get_sampler_index", [unit_userdata]);
    create_stub_callback(ctx, &mut magic, "shader_get_uniform", [unit_userdata]);
    create_stub_callback(ctx, &mut magic, "shader_set", []);
    create_stub_callback(ctx, &mut magic, "surface_exists", [true.into()]);
    create_stub_callback(ctx, &mut magic, "surface_get_width", [1366.into()]);
    create_stub_callback(ctx, &mut magic, "surface_get_height", [768.into()]);
    create_stub_callback(ctx, &mut magic, "surface_depth_disable", []);
    create_stub_callback(ctx, &mut magic, "surface_free", []);
    create_stub_callback(ctx, &mut magic, "surface_resize", []);
    create_stub_callback(
        ctx,
        &mut magic,
        "vertex_create_buffer_from_buffer_ext",
        [unit_userdata],
    );
    create_stub_callback(ctx, &mut magic, "vertex_format_add_color", []);
    create_stub_callback(ctx, &mut magic, "vertex_format_add_custom", []);
    create_stub_callback(ctx, &mut magic, "vertex_format_add_position_3d", []);
    create_stub_callback(ctx, &mut magic, "vertex_format_add_texcoord", []);
    create_stub_callback(ctx, &mut magic, "vertex_format_begin", []);
    create_stub_callback(ctx, &mut magic, "vertex_format_end", [unit_userdata]);
    create_stub_callback(ctx, &mut magic, "window_center", []);
    create_stub_callback(ctx, &mut magic, "window_has_focus", [true.into()]);
    create_stub_callback(ctx, &mut magic, "window_enable_borderless_fullscreen", []);
    create_stub_callback(ctx, &mut magic, "window_set_fullscreen", []);
    create_stub_callback(ctx, &mut magic, "window_set_size", []);
    create_stub_callback(ctx, &mut magic, "display_set_gui_size", []);
    create_stub_callback(ctx, &mut magic, "texture_prefetch", []);

    create_stub_constant(
        ctx,
        &mut magic,
        "view_camera",
        vm::Array::from_iter(&ctx, [unit_userdata; 8]),
    );
    create_stub_constant(
        ctx,
        &mut magic,
        "view_visible",
        vm::Array::from_iter(
            &ctx,
            [true, false, false, false, false, false, false, false]
                .into_iter()
                .map(|b| b.into()),
        ),
    );

    create_stub_constant(ctx, &mut magic, "cmpfunc_always", unit_userdata);
    create_stub_constant(ctx, &mut magic, "vertex_type_float1", unit_userdata);
    create_stub_constant(ctx, &mut magic, "vertex_type_float2", unit_userdata);
    create_stub_constant(ctx, &mut magic, "vertex_type_float3", unit_userdata);
    create_stub_constant(ctx, &mut magic, "vertex_type_float4", unit_userdata);
    create_stub_constant(ctx, &mut magic, "vertex_usage_textcoord", unit_userdata);

    create_stub_callback(
        ctx,
        &mut magic,
        "sprite_get_uvs",
        [vm::Array::from_iter(
            &ctx,
            [
                0.into(),
                0.into(),
                8.into(),
                8.into(),
                0.into(),
                0.into(),
                1.0.into(),
                1.0.into(),
            ],
        )
        .into()],
    );

    for key in [
        "vk_delete",
        "vk_end",
        "vk_pagedown",
        "vk_pageup",
        "vk_insert",
        "vk_space",
        "vk_tab",
        "vk_backspace",
        "vk_shift",
        "vk_control",
        "vk_enter",
        "vk_up",
        "vk_down",
        "vk_left",
        "vk_right",
        "vk_f1",
        "vk_f2",
        "vk_f3",
        "vk_f4",
        "vk_f5",
        "vk_f6",
        "vk_f7",
        "vk_f8",
        "vk_f9",
        "vk_f10",
        "vk_f11",
        "vk_f12",
        "vk_home",
        "vk_escape",
    ] {
        create_stub_constant(ctx, &mut magic, key, vm::Value::Integer(0));
    }

    for gp_button in [
        "gp_select",
        "gp_start",
        "gp_stickl",
        "gp_stickr",
        "gp_padu",
        "gp_padd",
        "gp_padl",
        "gp_padr",
        "gp_face4",
        "gp_face3",
        "gp_face2",
        "gp_face1",
        "gp_shoulderlb",
        "gp_shoulderrb",
        "gp_shoulderl",
        "gp_shoulderr",
        "gp_paddler",
        "gp_paddlel",
        "gp_paddlerb",
        "gp_paddlelb",
    ] {
        create_stub_constant(ctx, &mut magic, gp_button, 0);
    }

    for gp_axis in ["gp_axislv", "gp_axislh", "gp_axisrv", "gp_axisrh"] {
        create_stub_constant(ctx, &mut magic, gp_axis, 0);
    }

    create_stub_callback(ctx, &mut magic, "gamepad_is_connected", [false.into()]);

    create_stub_callback(ctx, &mut magic, "draw_set_font", [unit_userdata]);

    let font_get_info = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let mut data = vm::ObjectMap::new();
        let glyphs = vm::Object::new(&ctx);
        data.set_field(ctx, "glyphs", glyphs);
        exec.stack().replace(ctx, data);
        Ok(())
    });
    magic
        .add_constant(ctx, ctx.intern_static("font_get_info"), font_get_info)
        .unwrap();

    let string_width = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let string: vm::String = exec.stack().consume(ctx)?;
        let fake_width = string.chars().count() as f64 * 12.0;
        exec.stack().replace(ctx, fake_width);
        Ok(())
    });
    magic
        .add_constant(ctx, ctx.intern_static("string_width"), string_width)
        .unwrap();

    let string_width_ext = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let (_string, _line_height, width): (vm::String, f64, f64) = exec.stack().consume(ctx)?;
        exec.stack().replace(ctx, width);
        Ok(())
    });
    magic
        .add_constant(ctx, ctx.intern_static("string_width_ext"), string_width_ext)
        .unwrap();

    let string_height_ext = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let (string, line_height, width): (vm::String, f64, f64) = exec.stack().consume(ctx)?;
        let fake_width = string.chars().count() as f64 * 12.0;
        let fake_height = (fake_width / width).ceil() * line_height;
        exec.stack().replace(ctx, fake_height);
        Ok(())
    });
    magic
        .add_constant(
            ctx,
            ctx.intern_static("string_height_ext"),
            string_height_ext,
        )
        .unwrap();

    create_stub_callback(
        ctx,
        &mut magic,
        "layer_get_all_elements",
        [vm::Array::new(&ctx).into()],
    );

    create_stub_callback(ctx, &mut magic, "draw_self", []);

    create_stub_callback(ctx, &mut magic, "audio_falloff_set_model", []);
    create_stub_constant(
        ctx,
        &mut magic,
        "audio_falloff_linear_distance",
        unit_userdata,
    );
    create_stub_callback(ctx, &mut magic, "audio_listener_position", []);
    create_stub_callback(ctx, &mut magic, "audio_listener_orientation", []);
    create_stub_callback(ctx, &mut magic, "audio_play_sound_ext", []);

    create_stub_callback(ctx, &mut magic, "device_mouse_x_to_gui", [0.0.into()]);
    create_stub_callback(ctx, &mut magic, "device_mouse_y_to_gui", [0.0.into()]);
    create_stub_callback(
        ctx,
        &mut magic,
        "mouse_check_button_pressed",
        [false.into()],
    );
    create_stub_callback(
        ctx,
        &mut magic,
        "mouse_check_button_released",
        [false.into()],
    );
    create_stub_callback(ctx, &mut magic, "mouse_wheel_up", [false.into()]);
    create_stub_callback(ctx, &mut magic, "mouse_wheel_down", [false.into()]);
    create_stub_callback(
        ctx,
        &mut magic,
        "is_mouse_over_debug_overlay",
        [false.into()],
    );

    create_stub_callback(ctx, &mut magic, "keyboard_check", [false.into()]);
    create_stub_callback(ctx, &mut magic, "keyboard_check_pressed", [false.into()]);
    create_stub_callback(ctx, &mut magic, "keyboard_check_released", [false.into()]);
    create_stub_callback(
        ctx,
        &mut magic,
        "is_keyboard_used_debug_overlay",
        [false.into()],
    );

    magic
}
