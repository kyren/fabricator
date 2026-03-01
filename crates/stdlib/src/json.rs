use std::collections::HashSet;

use fabricator_vm as vm;
use gc_arena::Gc;

pub fn json_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    let json_parse = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let json: vm::String = exec.stack().consume(ctx)?;
        let value: serde_json::Value = serde_json::from_str(json.as_str())?;
        let value = json_to_value(ctx, value)?;
        exec.stack().push_back(value);
        Ok(())
    });
    lib.insert(
        ctx.intern("json_parse"),
        vm::MagicConstant::new_ptr(&ctx, json_parse),
    );

    let json_stringify = vm::Callback::from_fn(&ctx, |ctx, mut exec| {
        let value: vm::Value = exec.stack().consume(ctx)?;
        let json = value_to_json(ctx, &mut HashSet::new(), value)?;
        exec.stack().replace(ctx, serde_json::to_string(&json)?);
        Ok(())
    });
    lib.insert(
        ctx.intern("json_stringify"),
        vm::MagicConstant::new_ptr(&ctx, json_stringify),
    );
}

pub fn json_to_value<'gc>(
    ctx: vm::Context<'gc>,
    value: serde_json::Value,
) -> Result<vm::Value<'gc>, vm::RuntimeError> {
    Ok(match value {
        serde_json::Value::Null => vm::Value::Undefined,
        serde_json::Value::Bool(b) => vm::Value::Boolean(b),
        serde_json::Value::Number(number) => {
            if let Some(i) = number.as_i64() {
                vm::Value::Integer(i)
            } else if let Some(n) = number.as_f64() {
                vm::Value::Float(n)
            } else {
                return Err(vm::RuntimeError::msg(
                    "json number {number:?} is not an i64 or f64",
                ));
            }
        }
        serde_json::Value::String(s) => ctx.intern(&s).into(),
        serde_json::Value::Array(values) => {
            let array = vm::Array::new(&ctx);
            for value in values {
                array.push(&ctx, json_to_value(ctx, value)?);
            }
            vm::Value::Array(array)
        }
        serde_json::Value::Object(map) => {
            let obj = vm::Object::new(&ctx);
            for (key, value) in map {
                let key = ctx.intern(&key);
                let value = json_to_value(ctx, value)?;
                obj.set(&ctx, key, value);
            }
            vm::Value::Object(obj)
        }
    })
}

pub fn value_to_json<'gc>(
    ctx: vm::Context<'gc>,
    recursive_check: &mut HashSet<*const ()>,
    value: vm::Value<'gc>,
) -> Result<serde_json::Value, vm::RuntimeError> {
    Ok(match value {
        vm::Value::Undefined => serde_json::Value::Null,
        vm::Value::Boolean(b) => serde_json::Value::Bool(b),
        vm::Value::Integer(i) => serde_json::Value::Number(i.into()),
        vm::Value::Float(f) => serde_json::Value::Number(
            serde_json::Number::from_f64(f)
                .ok_or_else(|| vm::RuntimeError::msg("invalid JSON float value"))?,
        ),
        vm::Value::String(s) => serde_json::Value::String(s.as_str().to_owned()),
        vm::Value::Object(obj) => {
            if !recursive_check.insert(Gc::as_ptr(obj.into_inner()) as *const ()) {
                return Err(vm::RuntimeError::msg(
                    "cannot convert recursive object to JSON",
                ))?;
            }

            let mut map = serde_json::Map::new();
            let borrow = obj.borrow();
            for (&key, &value) in &borrow.map {
                map.insert(
                    key.as_str().to_owned(),
                    value_to_json(ctx, recursive_check, value)?,
                );
            }
            serde_json::Value::Object(map)
        }
        vm::Value::Array(arr) => {
            if !recursive_check.insert(Gc::as_ptr(arr.into_inner()) as *const ()) {
                return Err(vm::RuntimeError::msg(
                    "cannot convert recursive array to JSON",
                ))?;
            }

            let mut array = Vec::new();
            let borrow = arr.borrow();
            for &value in &*borrow {
                array.push(value_to_json(ctx, recursive_check, value)?);
            }
            serde_json::Value::Array(array)
        }
        vm::Value::UserData(ud) => {
            if let Some(s) = ud.coerce_string(ctx) {
                serde_json::Value::String(s.as_str().to_owned())
            } else if let Some(i) = ud.coerce_integer(ctx) {
                serde_json::Value::Number(i.into())
            } else if let Some(f) = ud.coerce_float(ctx) {
                serde_json::Value::Number(
                    serde_json::Number::from_f64(f)
                        .ok_or_else(|| vm::RuntimeError::msg("invalid JSON float value"))?,
                )
            } else {
                return Err(vm::RuntimeError::msg("cannot convert userdata to JSON"));
            }
        }
        vm::Value::Closure(_) | vm::Value::Callback(_) => {
            return Err(vm::RuntimeError::msg(format!(
                "cannot convert {} to JSON",
                value.type_name()
            )));
        }
    })
}
