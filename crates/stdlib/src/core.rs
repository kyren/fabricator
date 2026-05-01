use std::convert::Infallible;

use fabricator_vm as vm;

use crate::util::{MagicExt as _, Pointer, resolve_array_range};

/// `gml_pragma` is currently ignored by fabricator.
pub fn gml_pragma<'gc>(_ctx: vm::Context<'gc>, _args: ()) -> Result<(), vm::RuntimeError> {
    Ok(())
}

pub fn typeof_<'gc>(
    _ctx: vm::Context<'gc>,
    value: vm::Value<'gc>,
) -> Result<&'static str, vm::RuntimeError> {
    Ok(match value {
        vm::Value::Undefined => "undefined",
        vm::Value::Boolean(_) => "bool",
        vm::Value::Integer(_) => "int64",
        vm::Value::Float(_) => "number",
        vm::Value::String(_) => "string",
        vm::Value::Object(_) => "struct",
        vm::Value::Array(_) => "array",
        vm::Value::Closure(_) => "method",
        vm::Value::Callback(_) => "method",
        vm::Value::UserData(_) => "ptr",
    })
}

pub fn bool<'gc>(_ctx: vm::Context<'gc>, value: vm::Value<'gc>) -> Result<bool, Infallible> {
    Ok(value.cast_bool())
}

pub fn int64<'gc>(ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<i64, vm::RuntimeError> {
    if let Some(i) = arg.coerce_integer(ctx) {
        Ok(i)
    } else if let vm::Value::String(i) = arg {
        Ok(i.parse()?)
    } else {
        Err(vm::TypeError::new("number or string", arg.type_name()).into())
    }
}

pub fn real<'gc>(ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<f64, vm::TypeError> {
    arg.coerce_float(ctx)
        .ok_or_else(|| vm::TypeError::new("value coercible to float", arg.type_name()))
}

pub fn is_numeric<'gc>(_ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<bool, Infallible> {
    Ok(arg.cast_float().is_some())
}

pub fn is_real<'gc>(_ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<bool, Infallible> {
    Ok(arg.as_float().is_some() || arg.as_integer().is_some())
}

pub fn is_int64<'gc>(_ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<bool, Infallible> {
    Ok(arg.as_integer().is_some())
}

pub fn is_string<'gc>(_ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<bool, Infallible> {
    Ok(matches!(arg, vm::Value::String(_)))
}

pub fn is_struct<'gc>(_ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<bool, Infallible> {
    Ok(matches!(arg, vm::Value::Object(_)))
}

pub fn is_array<'gc>(_ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<bool, Infallible> {
    Ok(matches!(arg, vm::Value::Array(_)))
}

pub fn is_ptr<'gc>(_ctx: vm::Context<'gc>, arg: vm::Value<'gc>) -> Result<bool, Infallible> {
    Ok(matches!(arg, vm::Value::UserData(u) if Pointer::is_pointer(u)))
}

pub fn debug_get_callstack<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let max_depth: Option<usize> = exec.stack().consume(ctx)?;
    let frame_depth = exec.frame_depth();
    let max_depth = if let Some(max_depth) = max_depth {
        max_depth.min(frame_depth)
    } else {
        frame_depth
    };

    let array = vm::Array::new(&ctx);
    for frame in 0..max_depth {
        let frame_desc = match exec.upper_frame(frame) {
            vm::BacktraceFrame::Closure(closure_frame) => ctx.intern(&format!(
                "{}:{}",
                closure_frame.chunk_name(),
                closure_frame.line_number(),
            )),
            vm::BacktraceFrame::Callback(callback) => {
                ctx.intern(&vm::Value::Callback(callback).to_string())
            }
        };
        array.push(&ctx, frame_desc);
    }

    exec.stack().push_back(array);
    Ok(())
}

pub fn script_execute<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let mut func: vm::Function = exec.stack().from_index(ctx, 0)?;
    // `script_execute` is documented as calling the provided function in the *calling context*,
    // even if it is a bound method.
    if !func.this().is_undefined() {
        // NOTE: This allocates, to avoid this we could add a feature to call closures and
        // callbacks while ignoring any bound `this`.
        func = func.rebind(&ctx, vm::Value::Undefined);
    }
    Ok(exec.with_stack_bottom(1).call(ctx, func)?)
}

pub fn script_execute_ext<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let (mut func, args, offset, count): (
        vm::Function,
        Option<vm::Array>,
        Option<isize>,
        Option<isize>,
    ) = exec.stack().consume(ctx)?;
    if let Some(args) = args {
        let (range, reverse) = resolve_array_range(args.len(), offset, count)?;
        if reverse {
            for i in range.rev() {
                exec.stack().push_back(args.get(i).unwrap());
            }
        } else {
            for i in range {
                exec.stack().push_back(args.get(i).unwrap());
            }
        }
    }

    // `script_execute_ext` is documented as calling the provided function in the *calling
    // context*, even if it is a bound method.
    if !func.this().is_undefined() {
        // NOTE: This allocates, see the implementation of `script_execute`.
        func = func.rebind(&ctx, vm::Value::Undefined);
    }
    exec.call(ctx, func)?;
    Ok(())
}

pub fn method_call<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let (func, args, offset, count): (
        vm::Function,
        Option<vm::Array>,
        Option<isize>,
        Option<isize>,
    ) = exec.stack().consume(ctx)?;
    if let Some(args) = args {
        let (range, reverse) = resolve_array_range(args.len(), offset, count)?;
        if reverse {
            for i in range.rev() {
                exec.stack().push_back(args.get(i).unwrap());
            }
        } else {
            for i in range {
                exec.stack().push_back(args.get(i).unwrap());
            }
        }
    }
    exec.call(ctx, func)?;
    Ok(())
}

pub fn array_concat<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let array = vm::Array::new(&ctx);
    for i in 0..exec.stack().len() {
        let arr: vm::Array = exec.stack().from_index(ctx, i)?;
        array.extend(&ctx, arr.borrow().iter().copied());
    }
    exec.stack().replace(ctx, array);
    Ok(())
}

pub fn struct_get_names<'gc>(
    ctx: vm::Context<'gc>,
    obj: vm::Object<'gc>,
) -> Result<vm::Array<'gc>, Infallible> {
    Ok(vm::Array::from_iter(
        &ctx,
        obj.borrow().map.keys().map(|&k| k.into()),
    ))
}

pub fn struct_remove<'gc>(
    ctx: vm::Context<'gc>,
    (obj, key): (vm::Object<'gc>, vm::Value<'gc>),
) -> Result<(), vm::RuntimeError> {
    let key = key
        .coerce_string(ctx)
        .ok_or_else(|| vm::RuntimeError::msg("key not coercible to string"))?;
    obj.remove(&ctx, key);
    Ok(())
}

pub fn struct_exists<'gc>(
    ctx: vm::Context<'gc>,
    (obj, key): (vm::Object<'gc>, vm::Value<'gc>),
) -> Result<bool, vm::RuntimeError> {
    let key = key
        .coerce_string(ctx)
        .ok_or_else(|| vm::RuntimeError::msg("key not coercible to string"))?;
    Ok(obj.get(key).is_some())
}

pub fn struct_names_count<'gc>(
    _ctx: vm::Context<'gc>,
    obj: vm::Object<'gc>,
) -> Result<i64, Infallible> {
    Ok(obj.borrow().map.keys().len() as i64)
}

/// Gets a key from a struct.
///
/// This is the equivalent of doing `struct[$ "key"]`.
pub fn struct_get<'gc>(
    _ctx: vm::Context<'gc>,
    (obj, key): (vm::Object<'gc>, vm::String<'gc>),
) -> Result<vm::Value<'gc>, Infallible> {
    Ok(obj
        .borrow()
        .map
        .get(&key)
        .copied()
        .unwrap_or(vm::Value::Undefined))
}

pub fn core_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_constant(ctx, "pointer_null", Pointer::null().into_userdata(&ctx));
    lib.insert_callback(ctx, "gml_pragma", gml_pragma);
    lib.insert_callback(ctx, "typeof", typeof_);
    lib.insert_callback(ctx, "bool", bool);
    lib.insert_callback(ctx, "int64", int64);
    lib.insert_callback(ctx, "real", real);
    lib.insert_callback(ctx, "is_numeric", is_numeric);
    lib.insert_callback(ctx, "is_real", is_real);
    lib.insert_callback(ctx, "is_int64", is_int64);
    lib.insert_callback(ctx, "is_string", is_string);
    lib.insert_callback(ctx, "is_struct", is_struct);
    lib.insert_callback(ctx, "is_array", is_array);
    lib.insert_callback(ctx, "is_ptr", is_ptr);
    lib.insert_exec_callback(ctx, "debug_get_callstack", debug_get_callstack);
    lib.insert_exec_callback(ctx, "script_execute", script_execute);
    lib.insert_exec_callback(ctx, "script_execute_ext", script_execute_ext);
    lib.insert_exec_callback(ctx, "method_call", method_call);
    lib.insert_exec_callback(ctx, "array_concat", array_concat);
    lib.insert_callback(ctx, "struct_get_names", struct_get_names);
    lib.insert_callback(ctx, "struct_remove", struct_remove);
    lib.insert_callback(ctx, "struct_exists", struct_exists);
    lib.insert_callback(ctx, "struct_names_count", struct_names_count);
    lib.insert_callback(ctx, "struct_get", struct_get);
    lib.insert_callback(ctx, "variable_instance_get", struct_get);
}
