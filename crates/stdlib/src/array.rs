use std::{
    cmp::{self, Ordering},
    convert::Infallible,
    iter,
};

use fabricator_vm as vm;
use gc_arena::Gc;
use rand::RngExt as _;

use crate::util::{MagicExt as _, resolve_array_index, resolve_array_range};

pub fn array_create<'gc>(
    ctx: vm::Context<'gc>,
    (length, value): (usize, vm::Value<'gc>),
) -> Result<vm::Array<'gc>, Infallible> {
    Ok(vm::Array::from_iter(&ctx, iter::repeat_n(value, length)))
}

pub fn array_create_ext<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let (length, create): (usize, vm::Function) = exec.stack().consume(ctx)?;
    let array = vm::Array::with_capacity(&ctx, length);

    for i in 0..length {
        exec.stack().replace(ctx, i as isize);
        exec.call(ctx, create)?;
        array.set(&ctx, i, exec.stack().get(0));
    }

    exec.stack().replace(ctx, array);
    Ok(())
}

pub fn array_length<'gc>(
    _ctx: vm::Context<'gc>,
    array: vm::Array<'gc>,
) -> Result<isize, Infallible> {
    Ok(array.len() as isize)
}

pub fn array_delete<'gc>(
    ctx: vm::Context<'gc>,
    (array, index, count): (vm::Array<'gc>, isize, isize),
) -> Result<(), vm::RuntimeError> {
    let (range, _) = resolve_array_range(array.len(), Some(index), Some(count))?;
    array.borrow_mut(&ctx).drain(range);
    Ok(())
}

pub fn array_get_index<'gc>(
    _ctx: vm::Context<'gc>,
    (array, value, offset, length): (vm::Array<'gc>, vm::Value<'gc>, Option<isize>, Option<isize>),
) -> Result<isize, vm::RuntimeError> {
    let (range, is_reverse) = resolve_array_range(array.len(), offset, length)?;
    let array = array.borrow();
    let mut range_iter = array[range.clone()].iter();

    let idx = if is_reverse {
        range_iter
            .rev()
            .position(|&v| v == value)
            .map(|i| (range.end - 1 - i) as isize)
    } else {
        range_iter
            .position(|&v| v == value)
            .map(|i| (i + range.start) as isize)
    }
    .unwrap_or(-1);

    Ok(idx)
}

pub fn array_push<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::TypeError> {
    let array: vm::Array = exec.stack().from_index(ctx, 0)?;
    for &value in &exec.stack()[1..] {
        array.push(&ctx, value)
    }
    exec.stack().clear();
    Ok(())
}

pub fn array_pop<'gc>(
    ctx: vm::Context<'gc>,
    array: vm::Array<'gc>,
) -> Result<vm::Value<'gc>, Infallible> {
    Ok(array.pop(&ctx))
}

pub fn array_sort<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let (array, comparator): (vm::Array, Option<vm::Value>) = exec.stack().consume(ctx)?;
    sort_array(
        ctx,
        exec.reborrow(),
        array,
        comparator.unwrap_or(true.into()),
    )?;
    Ok(())
}

pub fn array_contains<'gc>(
    ctx: vm::Context<'gc>,
    (array, value, index, count): (vm::Array<'gc>, vm::Value<'gc>, Option<isize>, Option<isize>),
) -> Result<bool, vm::RuntimeError> {
    let (range, _) = resolve_array_range(array.len(), index, count)?;
    Ok(array.borrow_mut(&ctx)[range].contains(&value))
}

pub fn array_map<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let (input, function, index, count): (vm::Array, vm::Function, Option<isize>, Option<isize>) =
        exec.stack().consume(ctx)?;
    let (range, _) = resolve_array_range(input.len(), index, count)?;
    let output = vm::Array::with_capacity(&ctx, range.len());
    for i in range {
        exec.stack().replace(ctx, (input.get(i), i as isize));
        exec.call(ctx, function)?;
        output.set(&ctx, i, exec.stack().get(0));
    }
    exec.stack().replace(ctx, output);
    Ok(())
}

pub fn array_copy<'gc>(
    ctx: vm::Context<'gc>,
    (dest, dest_index, src, src_index, length): (
        vm::Array<'gc>,
        isize,
        vm::Array<'gc>,
        isize,
        isize,
    ),
) -> Result<(), vm::RuntimeError> {
    let dest_index = resolve_array_index(dest.len(), Some(dest_index))?;
    let (src_range, is_reverse) = resolve_array_range(src.len(), Some(src_index), Some(length))?;

    if is_reverse {
        for (i, ind) in src_range.rev().enumerate() {
            dest.set(&ctx, dest_index + i, src.get(ind));
        }
    } else {
        for (i, ind) in src_range.enumerate() {
            dest.set(&ctx, dest_index + i, src.get(ind));
        }
    }
    Ok(())
}

pub fn array_resize<'gc>(
    ctx: vm::Context<'gc>,
    (array, new_len): (vm::Array<'gc>, usize),
) -> Result<(), Infallible> {
    array.borrow_mut(&ctx).resize(new_len, vm::Value::Undefined);
    Ok(())
}

/// Insert a value into an array at a given position, shifting all other values
/// down one index.
///
/// If [`array_length`] is less than `pos`, we will resize the array
/// to be at least as long as `pos + 1`. If we expand the array in such a manner,
/// the intervening slots will be filled with [`vm::Value::Undefined`], as if you
/// had called [`array_resize`] with `pos + 1`.
pub fn array_insert<'gc>(
    ctx: vm::Context<'gc>,
    (array, pos, value): (vm::Array<'gc>, usize, vm::Value<'gc>),
) -> Result<(), Infallible> {
    let mut a = array.borrow_mut(&ctx);
    if pos > a.len() {
        a.resize(pos + 1, vm::Value::Undefined);
    }
    a.insert(pos, value);

    Ok(())
}

/// Checks if any member of the array satisfies the given function. This function
/// short-circuits, so once it returns true once, then it will be stop running.
///
/// In the event that the array itself is shortened by a closer passed into this function,
/// which is very bad idea, this closure will give the value `vm::Value::Undefined` for indices
/// which no longer exist on the array.
pub fn array_any<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let (input, function): (vm::Array<'gc>, vm::Function<'gc>) = exec.stack().consume(ctx)?;

    let len = input.len();
    let mut o = false;
    for i in 0..len {
        exec.stack().replace(ctx, (input.get(i), i as isize));
        exec.call(ctx, function)?;
        if exec.stack().get(0).cast_bool() {
            o = true;
            break;
        }
    }

    exec.stack().replace(ctx, vm::Value::Boolean(o));
    Ok(())
}

pub fn array_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_callback(ctx, "array_create", array_create);
    lib.insert_exec_callback(ctx, "array_create_ext", array_create_ext);
    lib.insert_callback(ctx, "array_length", array_length);
    lib.insert_callback(ctx, "array_delete", array_delete);
    lib.insert_callback(ctx, "array_get_index", array_get_index);
    lib.insert_exec_callback(ctx, "array_push", array_push);
    lib.insert_callback(ctx, "array_pop", array_pop);
    lib.insert_exec_callback(ctx, "array_sort", array_sort);
    lib.insert_callback(ctx, "array_contains", array_contains);
    lib.insert_exec_callback(ctx, "array_map", array_map);
    lib.insert_callback(ctx, "array_copy", array_copy);
    lib.insert_callback(ctx, "array_resize", array_resize);
    lib.insert_callback(ctx, "array_insert", array_insert);
    lib.insert_exec_callback(ctx, "array_any", array_any);
}

fn sort_array<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
    array: vm::Array<'gc>,
    comparator: vm::Value<'gc>,
) -> Result<(), vm::RuntimeError> {
    #[derive(Copy, Clone)]
    enum SortBy<'gc> {
        Ascending,
        Descending,
        Custom(vm::Function<'gc>),
    }

    fn value_cmp<'gc>(lhs: vm::Value<'gc>, rhs: vm::Value<'gc>) -> cmp::Ordering {
        // The GMS2 documentation for `array_sort` barely covers what the "default sort order" is,
        // but we must make a total order for all values.

        #[derive(Copy, Clone)]
        struct TotalNum(f64);

        impl Ord for TotalNum {
            fn cmp(&self, other: &Self) -> Ordering {
                self.0.total_cmp(&other.0)
            }
        }

        impl PartialEq for TotalNum {
            fn eq(&self, other: &Self) -> bool {
                self.cmp(other).is_eq()
            }
        }

        impl Eq for TotalNum {}

        impl PartialOrd for TotalNum {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        // We categorize values into four types and sort them independently. This does *not* treat
        // numbers "stringly", which seems to match the GMS2 documentation which states:
        //
        //   If the array contains a set of strings, then the strings will be sorted alphabetically
        //   based on the English alphabet when using the default ascending/descending sort type.
        //   All other data types will be sorted based on their numerical value, the exact values
        //   of which will depend on the data type itself.
        //
        // All strings will be sorted before all numerical scalars which will be sorted before all
        // heap values which will be sorted before any instances of `undefined`.
        #[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
        enum SortValue<'a> {
            String(&'a str),
            Numeric(TotalNum),
            Pointer(*const ()),
            Undefined,
        }

        fn to_sort_value<'gc>(value: vm::Value<'gc>) -> SortValue<'gc> {
            match value {
                vm::Value::Undefined => SortValue::Undefined,
                vm::Value::Boolean(_) | vm::Value::Integer(_) | vm::Value::Float(_) => {
                    SortValue::Numeric(TotalNum(value.cast_float().unwrap()))
                }
                vm::Value::String(s) => SortValue::String(s.as_str()),
                vm::Value::Object(o) => SortValue::Pointer(Gc::as_ptr(o.into_inner()) as *const ()),
                vm::Value::Array(a) => SortValue::Pointer(Gc::as_ptr(a.into_inner()) as *const ()),
                vm::Value::Closure(c) => {
                    SortValue::Pointer(Gc::as_ptr(c.into_inner()) as *const ())
                }
                vm::Value::Callback(c) => {
                    SortValue::Pointer(Gc::as_ptr(c.into_inner()) as *const ())
                }
                vm::Value::UserData(u) => {
                    SortValue::Pointer(Gc::as_ptr(u.into_inner()) as *const ())
                }
            }
        }

        to_sort_value(lhs).cmp(&to_sort_value(rhs))
    }

    fn cmp_by<'gc>(
        ctx: vm::Context<'gc>,
        exec: &mut vm::Execution<'gc, '_>,
        sort_by: SortBy<'gc>,
        lhs: vm::Value<'gc>,
        rhs: vm::Value<'gc>,
    ) -> Result<cmp::Ordering, vm::RuntimeError> {
        Ok(match sort_by {
            SortBy::Ascending => value_cmp(lhs, rhs),
            SortBy::Descending => value_cmp(rhs, lhs),
            SortBy::Custom(func) => {
                exec.stack().replace(ctx, (lhs, rhs));
                exec.call(ctx, func)?;
                let n: f64 = exec.stack().consume(ctx)?;
                if n < 0.0 {
                    cmp::Ordering::Less
                } else if n > 0.0 {
                    cmp::Ordering::Greater
                } else if n == 0.0 {
                    cmp::Ordering::Equal
                } else {
                    return Err(vm::RuntimeError::msg(
                        "numeric value returned by comparator is NaN",
                    ));
                }
            }
        })
    }

    fn partition<'gc>(
        ctx: vm::Context<'gc>,
        exec: &mut vm::Execution<'gc, '_>,
        sort_by: SortBy<'gc>,
        array: &mut [vm::Value<'gc>],
        pivot: usize,
    ) -> Result<usize, vm::RuntimeError> {
        let last = array.len() - 1;
        array.swap(pivot, last);
        let mut i = 0;
        for j in 0..last {
            if cmp_by(ctx, exec, sort_by, array[j], array[last])?.is_lt() {
                array.swap(i, j);
                i += 1;
            }
        }
        array.swap(i, last);
        Ok(i)
    }

    fn quicksort<'gc>(
        ctx: vm::Context<'gc>,
        exec: &mut vm::Execution<'gc, '_>,
        sort_by: SortBy<'gc>,
        rng: &mut impl rand::Rng,
        array: &mut [vm::Value<'gc>],
    ) -> Result<(), vm::RuntimeError> {
        if array.len() <= 1 {
            return Ok(());
        }

        let pivot = partition(ctx, exec, sort_by, array, rng.random_range(0..array.len()))?;
        let (left, right) = array.split_at_mut(pivot);

        quicksort(ctx, exec, sort_by, rng, left)?;
        quicksort(ctx, exec, sort_by, rng, &mut right[1..])?;
        Ok(())
    }

    let sort_by = if let Some(func) = comparator.as_function() {
        SortBy::Custom(func)
    } else if comparator.cast_bool() {
        SortBy::Ascending
    } else {
        SortBy::Descending
    };

    quicksort(
        ctx,
        &mut exec,
        sort_by,
        &mut rand::rng(),
        &mut array.borrow_mut(&ctx),
    )
}
