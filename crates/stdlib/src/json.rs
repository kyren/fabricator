use std::collections::HashSet;

use fabricator_vm as vm;
use gc_arena::Gc;
use thiserror::Error;

use crate::util::MagicExt as _;

pub fn json_to_value<'gc>(
    ctx: vm::Context<'gc>,
    value: serde_json::Value,
) -> Result<vm::Value<'gc>, FromJsonError> {
    match value {
        serde_json::Value::Null => Ok(vm::Value::Undefined),
        serde_json::Value::Bool(b) => Ok(vm::Value::Boolean(b)),
        serde_json::Value::Number(number) => {
            if let Some(i) = number.as_i64() {
                Ok(vm::Value::Integer(i))
            } else if let Some(n) = number.as_f64() {
                Ok(vm::Value::Float(n))
            } else {
                Err(FromJsonError::BadNumber(number))
            }
        }
        serde_json::Value::String(s) => Ok(ctx.intern(&s).into()),
        serde_json::Value::Array(values) => {
            let mut array = vm::ArrayVec::new();
            for value in values {
                array.push(json_to_value(ctx, value)?);
            }
            Ok(vm::Value::Array(vm::Array::from_vec(&ctx, array)))
        }
        serde_json::Value::Object(map) => {
            let mut obj = vm::ObjectMap::new();
            for (key, value) in map {
                let key = ctx.intern(&key);
                let value = json_to_value(ctx, value)?;
                obj.set(key, value);
            }
            Ok(vm::Value::Object(vm::Object::with_parts(&ctx, obj, None)))
        }
    }
}

#[derive(Debug, Error)]
pub enum FromJsonError {
    #[error("{0:?} is not a valid i64 or f64")]
    BadNumber(serde_json::Number),
}

pub fn json_parse<'gc>(
    ctx: vm::Context<'gc>,
    json: vm::String<'gc>,
) -> Result<vm::Value<'gc>, vm::RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(json.as_str())?;
    Ok(json_to_value(ctx, value)?)
}

pub fn value_to_json<'gc>(
    ctx: vm::Context<'gc>,
    recursive_check: &mut HashSet<*const ()>,
    value: vm::Value<'gc>,
) -> Result<serde_json::Value, ToJsonError> {
    match value {
        vm::Value::Undefined => Ok(serde_json::Value::Null),
        vm::Value::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        vm::Value::Integer(i) => Ok(serde_json::Value::Number(i.into())),
        vm::Value::Float(f) => Ok(serde_json::Value::Number(
            serde_json::Number::from_f64(f).ok_or_else(|| ToJsonError::NumberNotFinite)?,
        )),
        vm::Value::String(s) => Ok(serde_json::Value::String(s.as_str().to_owned())),
        vm::Value::Object(obj) => {
            let obj_ptr = Gc::as_ptr(obj.into_inner()) as *const ();
            if !recursive_check.insert(obj_ptr) {
                return Err(ToJsonError::Recursive("Object"))?;
            }

            let mut map = serde_json::Map::new();
            let borrow = obj
                .try_borrow()
                .map_err(|_| ToJsonError::BorrowError("Object"))?;
            for (key, value) in &*borrow {
                map.insert(
                    key.as_str().to_owned(),
                    value_to_json(ctx, recursive_check, value)?,
                );
            }

            recursive_check.remove(&obj_ptr);
            Ok(serde_json::Value::Object(map))
        }
        vm::Value::Array(arr) => {
            let arr_ptr = Gc::as_ptr(arr.into_inner()) as *const ();
            if !recursive_check.insert(arr_ptr) {
                return Err(ToJsonError::Recursive("Array"))?;
            }

            let mut array = Vec::new();
            let borrow = arr
                .try_borrow()
                .map_err(|_| ToJsonError::BorrowError("Array"))?;
            for value in &*borrow {
                array.push(value_to_json(ctx, recursive_check, value)?);
            }

            recursive_check.remove(&arr_ptr);
            Ok(serde_json::Value::Array(array))
        }
        vm::Value::UserData(ud) => {
            if let Some(s) = ud.coerce_string(ctx) {
                Ok(serde_json::Value::String(s.as_str().to_owned()))
            } else if let Some(i) = ud.coerce_integer(ctx) {
                Ok(serde_json::Value::Number(i.into()))
            } else if let Some(f) = ud.coerce_float(ctx) {
                Ok(serde_json::Value::Number(
                    serde_json::Number::from_f64(f).ok_or_else(|| ToJsonError::NumberNotFinite)?,
                ))
            } else {
                Err(ToJsonError::InvalidType("UserData"))
            }
        }
        vm::Value::Closure(_) => Err(ToJsonError::InvalidType("Closure")),
        vm::Value::Callback(_) => Err(ToJsonError::InvalidType("Callback")),
    }
}

#[derive(Debug, Error)]
pub enum ToJsonError {
    #[error("cannot convert recursive {0} to JSON")]
    Recursive(&'static str),
    #[error("`{0}` is already borrowed mutably")]
    BorrowError(&'static str),
    #[error("`{0}` cannot be converted to JSON")]
    InvalidType(&'static str),
    #[error("cannot convert non-finite number to JSON")]
    NumberNotFinite,
}

pub fn json_stringify<'gc>(
    ctx: vm::Context<'gc>,
    value: vm::Value<'gc>,
) -> Result<String, vm::RuntimeError> {
    let json = value_to_json(ctx, &mut HashSet::new(), value)?;
    Ok(serde_json::to_string(&json)?)
}

pub fn json_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_callback(ctx, "json_parse", json_parse);
    lib.insert_callback(ctx, "json_stringify", json_stringify);
}
