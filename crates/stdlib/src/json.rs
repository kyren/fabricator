use std::collections::HashSet;

use fabricator_vm as vm;
use gc_arena::Gc;

use crate::util::MagicExt as _;

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
                return Err(vm::RuntimeError::msg(format!(
                    "json number {number:?} is not an i64 or f64"
                )));
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

pub fn json_parse<'gc>(
    ctx: vm::Context<'gc>,
    json: vm::String<'gc>,
) -> Result<vm::Value<'gc>, vm::RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(json.as_str())?;
    json_to_value(ctx, value)
}

pub fn value_to_json<'gc>(
    ctx: vm::Context<'gc>,
    recursive_check: &mut HashSet<*const ()>,
    value: vm::Value<'gc>,
) -> Result<serde_json::Value, ToJsonErr> {
    let output = match value {
        vm::Value::Undefined => serde_json::Value::Null,
        vm::Value::Boolean(b) => serde_json::Value::Bool(b),
        vm::Value::Integer(i) => serde_json::Value::Number(i.into()),
        vm::Value::Float(f) => serde_json::Value::Number(
            serde_json::Number::from_f64(f).ok_or(ToJsonErr::InvalidNumericalRepresentation(f))?,
        ),
        vm::Value::String(s) => serde_json::Value::String(s.as_str().to_owned()),
        vm::Value::Object(obj) => {
            let ptr = Gc::as_ptr(obj.into_inner()) as *const ();
            if !recursive_check.insert(ptr) {
                return Err(ToJsonErr::Recursive("object"));
            }

            let mut map = serde_json::Map::new();
            let borrow = obj.borrow();
            for (&key, &value) in &borrow.map {
                map.insert(
                    key.as_str().to_owned(),
                    value_to_json(ctx, recursive_check, value)?,
                );
            }
            recursive_check.remove(&ptr);

            serde_json::Value::Object(map)
        }
        vm::Value::Array(arr) => {
            let ptr = Gc::as_ptr(arr.into_inner()) as *const ();
            if !recursive_check.insert(ptr) {
                return Err(ToJsonErr::Recursive("array"));
            }

            let mut array = Vec::new();
            let borrow = arr.borrow();
            for &value in &*borrow {
                array.push(value_to_json(ctx, recursive_check, value)?);
            }
            recursive_check.remove(&ptr);
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
                        .ok_or(ToJsonErr::InvalidNumericalRepresentation(f))?,
                )
            } else {
                return Err(ToJsonErr::InvalidConversion("userdata"));
            }
        }
        vm::Value::Closure(_) | vm::Value::Callback(_) => {
            return Err(ToJsonErr::InvalidConversion(value.type_name()));
        }
    };

    Ok(output)
}

pub fn json_stringify<'gc>(
    ctx: vm::Context<'gc>,
    value: vm::Value<'gc>,
) -> Result<String, vm::RuntimeError> {
    let json = value_to_json(ctx, &mut HashSet::new(), value)?;
    Ok(serde_json::to_string(&json)?)
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum ToJsonErr {
    #[error("cannot convert recursive {0} to JSON")]
    Recursive(&'static str),

    #[error("cannot convert {0} to JSON")]
    InvalidConversion(&'static str),

    // this basically is `infinity` or `NaN`, which JSON disallows
    #[error("invalid JSON number {0}")]
    InvalidNumericalRepresentation(f64),
}

pub fn json_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_callback(ctx, "json_parse", json_parse);
    lib.insert_callback(ctx, "json_stringify", json_stringify);
}
