use std::{
    collections::HashSet,
    convert::Infallible,
    fmt,
    io::{self, Write as _},
    mem,
};

use fabricator_vm as vm;
use gc_arena::Gc;

use crate::util::{MagicExt as _, resolve_array_range};

pub fn string_trim<'gc>(
    ctx: vm::Context<'gc>,
    (string, trims): (vm::String<'gc>, Option<Vec<vm::String<'gc>>>),
) -> Result<vm::String<'gc>, Infallible> {
    Ok(if let Some(trims) = trims {
        let mut string = string.as_str();
        for trim in trims {
            string = string.trim_start_matches(trim.as_str());
            string = string.trim_end_matches(trim.as_str());
        }
        ctx.intern(string)
    } else {
        ctx.intern(string.as_str().trim())
    })
}

pub fn string_length<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::VmError<'gc>> {
    let value: vm::Value = exec.stack().consume(ctx)?;
    let string = value_to_string(ctx, exec.reborrow(), value)?;
    exec.stack().replace(ctx, string.chars().count() as isize);
    Ok(())
}

pub fn string_byte_length<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::VmError<'gc>> {
    let value: vm::Value = exec.stack().consume(ctx)?;
    let string = value_to_string(ctx, exec.reborrow(), value)?;
    exec.stack().replace(ctx, string.len() as isize);
    Ok(())
}

pub fn ord<'gc>(_ctx: vm::Context<'gc>, arg: vm::String<'gc>) -> Result<isize, vm::RuntimeError> {
    let mut iter = arg.as_str().chars();
    let c = iter.next();
    if c.is_none() || iter.next().is_some() {
        return Err(vm::RuntimeError::msg(
            "`ord` must be given a single character string",
        ));
    }
    Ok(c.unwrap() as isize)
}

pub fn show_debug_message<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::VmError<'gc>> {
    let fmt_string: vm::String = exec.stack().from_index(ctx, 0)?;

    let mut stdout = io::stdout().lock();
    for part in split_format(&fmt_string) {
        match part {
            FormatPart::Str(s) => write!(stdout, "{}", s)?,
            FormatPart::Arg(arg) => {
                let mut buf = String::new();
                let val = exec.stack().get(arg + 1);
                print_value(&mut buf, ctx, exec.reborrow(), val)?;
                write!(stdout, "{}", buf)?;
            }
        }
    }
    writeln!(stdout)?;
    exec.stack().clear();
    Ok(())
}

pub fn string<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::VmError<'gc>> {
    let out = match exec.stack().get(0) {
        vm::Value::String(fmt) => {
            let mut out = String::new();
            for part in split_format(&fmt) {
                match part {
                    FormatPart::Str(s) => out.push_str(s),
                    FormatPart::Arg(arg) => {
                        let val = exec.stack().get(arg + 1);
                        print_value(&mut out, ctx, exec.reborrow(), val)?;
                    }
                }
            }
            ctx.intern(&out)
        }
        other => value_to_string(ctx, exec.reborrow(), other)?,
    };
    exec.stack().replace(ctx, out);
    Ok(())
}

/// Gets the character at a given character index in the string.
/// Strings are 1-indexed, not 0-indexed.
///
/// If `index` is 0 (or a negative number) is provided, we'll return the first character in the string.
///
/// If `index` is greater than the length of the string (since we are 1-indexed, the last character
/// in the string), then we will return an *empty string*.
pub fn string_char_at<'gc>(
    _ctx: vm::Context<'gc>,
    (string, index): (vm::String<'gc>, i64),
) -> Result<String, Infallible> {
    let mut chars = string.chars();
    let index = if index <= 0 { 1 } else { index as usize - 1 };
    Ok(chars.nth(index).map(|v| v.to_string()).unwrap_or_default())
}

pub fn string_digits<'gc>(
    _ctx: vm::Context<'gc>,
    input: vm::String<'gc>,
) -> Result<String, Infallible> {
    let mut output = String::new();

    for c in input.chars() {
        if c.is_ascii_digit() {
            output.push(c);
        }
    }

    Ok(output)
}

pub fn string_pos<'gc>(
    _ctx: vm::Context<'gc>,
    (substr, string): (vm::String<'gc>, vm::String<'gc>),
) -> Result<isize, Infallible> {
    Ok(string
        .find(substr.as_str())
        .map(|byte_pos| {
            string
                .char_indices()
                .position(|(bp, _)| byte_pos == bp)
                .unwrap() as isize
                + 1
        })
        .unwrap_or(0))
}

pub fn string_last_pos<'gc>(
    _ctx: vm::Context<'gc>,
    (substr, string): (vm::String<'gc>, vm::String<'gc>),
) -> Result<isize, Infallible> {
    Ok(string
        .rfind(substr.as_str())
        .map(|byte_pos| {
            string
                .char_indices()
                .position(|(bp, _)| byte_pos == bp)
                .unwrap() as isize
                + 1
        })
        .unwrap_or(0))
}

pub fn string_count<'gc>(
    _ctx: vm::Context<'gc>,
    (substr, string): (vm::String<'gc>, vm::String<'gc>),
) -> Result<isize, vm::RuntimeError> {
    if substr.is_empty() {
        return Err(vm::RuntimeError::msg(
            "substr cannot be empty in `string_count`",
        ));
    }
    let mut string = string.as_str();
    let mut count = 0;
    while let Some(index) = string.find(substr.as_str()) {
        count += 1;
        string = &string[index + substr.len()..];
    }
    Ok(count)
}

pub fn string_copy<'gc>(
    _ctx: vm::Context<'gc>,
    (string, index, count): (vm::String<'gc>, usize, usize),
) -> Result<String, vm::RuntimeError> {
    let index = index.checked_sub(1).ok_or_else(|| {
        vm::RuntimeError::msg("index given to `string_copy` is 1-indexed and cannot be 0")
    })?;
    Ok(string.chars().skip(index).take(count).collect::<String>())
}

pub fn string_delete<'gc>(
    _ctx: vm::Context<'gc>,
    (string, index, count): (vm::String<'gc>, isize, isize),
) -> Result<String, vm::RuntimeError> {
    if index == 0 {
        return Err(vm::RuntimeError::msg(
            "index given to `string_delete` is 1-indexed and cannot be 0",
        ));
    }
    let (range, _) = resolve_array_range(string.chars().count(), Some(index - 1), Some(count))?;
    Ok(string
        .chars()
        .enumerate()
        .filter_map(|(i, c)| if range.contains(&i) { None } else { Some(c) })
        .collect::<String>())
}

pub fn string_insert<'gc>(
    _ctx: vm::Context<'gc>,
    (substr, string, index): (vm::String<'gc>, vm::String<'gc>, usize),
) -> Result<String, Infallible> {
    let index = index.saturating_sub(1).clamp(0, string.len());
    Ok(format!(
        "{}{}{}",
        &string[0..index],
        substr,
        &string[index..]
    ))
}

pub fn string_replace<'gc>(
    _ctx: vm::Context<'gc>,
    (string, substr, newstr): (vm::String<'gc>, vm::String<'gc>, vm::String<'gc>),
) -> Result<String, Infallible> {
    Ok(string.replacen(substr.as_str(), newstr.as_str(), 1))
}

pub fn string_replace_all<'gc>(
    _ctx: vm::Context<'gc>,
    (string, substr, newstr): (vm::String<'gc>, vm::String<'gc>, vm::String<'gc>),
) -> Result<String, Infallible> {
    Ok(string.replace(substr.as_str(), newstr.as_str()))
}

pub fn string_ends_with<'gc>(
    _ctx: vm::Context<'gc>,
    (string, substr): (vm::String<'gc>, vm::String<'gc>),
) -> Result<bool, Infallible> {
    Ok(string.ends_with(substr.as_str()))
}

pub fn string_trim_end<'gc>(
    ctx: vm::Context<'gc>,
    (string, patterns): (vm::String<'gc>, Option<vm::Array<'gc>>),
) -> Result<vm::String<'gc>, vm::RuntimeError> {
    let res = if let Some(patterns) = patterns {
        let patterns = patterns.try_borrow()?;
        let mut res = string.as_str();
        loop {
            let mut changed = false;
            for i in 0..patterns.len() {
                let pat = patterns
                    .get(i)
                    .unwrap()
                    .coerce_string(ctx)
                    .ok_or_else(|| vm::RuntimeError::msg("trim pattern must be a string"))?;
                let new_res = res.trim_end_matches(pat.as_str());
                if new_res.len() != res.len() {
                    changed = true;
                    res = new_res;
                }
            }
            if !changed {
                break;
            }
        }
        res
    } else {
        string.trim_end()
    };
    Ok(ctx.intern(res))
}

pub fn string_format<'gc>(
    _ctx: vm::Context<'gc>,
    (val, whole, decimal): (f64, usize, usize),
) -> Result<String, Infallible> {
    let decimal_point = if val.fract() == 0.0 { 0 } else { 1 };
    Ok(format!(
        "{val:width$.prec$}",
        val = val,
        width = whole + decimal + decimal_point,
        prec = decimal
    ))
}

pub fn string_lower<'gc>(
    _ctx: vm::Context<'gc>,
    string: vm::String<'gc>,
) -> Result<String, Infallible> {
    Ok(string.to_ascii_lowercase())
}

pub fn string_upper<'gc>(
    _ctx: vm::Context<'gc>,
    string: vm::String<'gc>,
) -> Result<String, Infallible> {
    Ok(string.to_ascii_uppercase())
}

pub fn string_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_callback(ctx, "string_trim", string_trim);
    lib.insert_exec_callback(ctx, "string_length", string_length);
    lib.insert_exec_callback(ctx, "string_byte_length", string_byte_length);
    lib.insert_callback(ctx, "ord", ord);
    lib.insert_exec_callback(ctx, "show_debug_message", show_debug_message);
    lib.insert_exec_callback(ctx, "string", string);
    lib.insert_callback(ctx, "string_char_at", string_char_at);
    lib.insert_callback(ctx, "string_digits", string_digits);
    lib.insert_callback(ctx, "string_pos", string_pos);
    lib.insert_callback(ctx, "string_last_pos", string_last_pos);
    lib.insert_callback(ctx, "string_count", string_count);
    lib.insert_callback(ctx, "string_copy", string_copy);
    lib.insert_callback(ctx, "string_delete", string_delete);
    lib.insert_callback(ctx, "string_insert", string_insert);
    lib.insert_callback(ctx, "string_replace", string_replace);
    lib.insert_callback(ctx, "string_replace_all", string_replace_all);
    lib.insert_callback(ctx, "string_ends_with", string_ends_with);
    lib.insert_callback(ctx, "string_trim_end", string_trim_end);
    lib.insert_callback(ctx, "string_format", string_format);
    lib.insert_callback(ctx, "string_lower", string_lower);
    lib.insert_callback(ctx, "string_upper", string_upper);
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FormatPart<'a> {
    Str(&'a str),
    Arg(usize),
}

pub fn split_format<'a>(s: &'a str) -> impl Iterator<Item = FormatPart<'a>> + 'a {
    struct Iter<'a> {
        rest: &'a str,
        next_arg: Option<usize>,
    }

    impl<'a> Iterator for Iter<'a> {
        type Item = FormatPart<'a>;

        fn next(&mut self) -> Option<Self::Item> {
            if let Some(next_arg) = self.next_arg.take() {
                return Some(FormatPart::Arg(next_arg));
            } else if self.rest.is_empty() {
                return None;
            }

            // Try to find a curly brace pair `{xxx}` where `xxx` is a valid `usize`. This is
            // interpreted as a "format argument" with the argument index between the braces.
            //
            // Parsing is completely forgiving because there is no actual specification for `string`
            // and `show_debug_message`. Anything that can be parsed as a valid format arg is,
            // anything else is left in the resulting string.

            let mut find_start_pos = 0;
            loop {
                let Some(left_brace_pos) = self.rest[find_start_pos..].find('{') else {
                    break;
                };
                let left_brace_pos = find_start_pos + left_brace_pos;

                // Stop at any trailing `{` so that we make sure to parse the innermost brace pair.
                //
                // The string `"hello {{0}"` should have one valid format argument.
                let Some(right_brace_pos) = self.rest[left_brace_pos + 1..].find(['{', '}']) else {
                    break;
                };
                let right_brace_pos = left_brace_pos + 1 + right_brace_pos;

                if self.rest[right_brace_pos..].starts_with('}') {
                    let leading = &self.rest[0..left_brace_pos];
                    let trailing = &self.rest[right_brace_pos + 1..];
                    if let Ok(arg_index) =
                        self.rest[left_brace_pos + 1..right_brace_pos].parse::<usize>()
                    {
                        self.rest = trailing;
                        return if leading.is_empty() {
                            Some(FormatPart::Arg(arg_index))
                        } else {
                            self.next_arg = Some(arg_index);
                            Some(FormatPart::Str(leading))
                        };
                    } else {
                        // Try again following the `}`.
                        find_start_pos = right_brace_pos + 1;
                    }
                } else {
                    // Try again at the `{`.
                    find_start_pos = right_brace_pos;
                }
            }

            Some(FormatPart::Str(mem::take(&mut self.rest)))
        }
    }

    Iter {
        rest: s,
        next_arg: None,
    }
}

/// Print any [`vm::Value`].
pub fn raw_print_value<'gc>(
    f: &mut dyn fmt::Write,
    ctx: vm::Context<'gc>,
    value: vm::Value<'gc>,
) -> Result<(), fmt::Error> {
    if let vm::Value::String(s) = value {
        write!(f, "{}", s)
    } else {
        pretty_print_value(f, ctx, None, value).map_err(|e| match e {
            PrintValueError::ToStringError(_) => unreachable!(),
            PrintValueError::FmtError(fmt_error) => fmt_error,
        })
    }
}

/// Print any [`vm::Value`] into a [`vm::String`].
pub fn raw_value_to_string<'gc>(ctx: vm::Context<'gc>, value: vm::Value<'gc>) -> vm::String<'gc> {
    if let vm::Value::String(s) = value {
        s
    } else {
        let mut s = String::new();
        pretty_print_value(&mut s, ctx, None, value).unwrap();
        ctx.intern(&s)
    }
}

#[derive(Debug)]
pub enum PrintValueError<'gc> {
    ToStringError(vm::VmError<'gc>),
    FmtError(fmt::Error),
}

impl<'gc> From<vm::VmError<'gc>> for PrintValueError<'gc> {
    fn from(err: vm::VmError<'gc>) -> Self {
        Self::ToStringError(err)
    }
}

impl<'gc> From<fmt::Error> for PrintValueError<'gc> {
    fn from(err: fmt::Error) -> Self {
        Self::FmtError(err)
    }
}

impl<'gc> PrintValueError<'gc> {
    fn into_vm_err(self) -> vm::VmError<'gc> {
        match self {
            PrintValueError::ToStringError(vm_error) => vm_error,
            PrintValueError::FmtError(error) => error.into(),
        }
    }
}

/// Print any [`vm::Value`], calling `toString` methods on objects if present.
///
/// On return, the stack in the provided `exec` is restored to its initial state.
pub fn print_value<'gc>(
    f: &mut dyn fmt::Write,
    ctx: vm::Context<'gc>,
    exec: vm::Execution<'gc, '_>,
    value: vm::Value<'gc>,
) -> Result<(), vm::VmError<'gc>> {
    if let vm::Value::String(s) = value {
        Ok(write!(f, "{}", s)?)
    } else {
        pretty_print_value(f, ctx, Some(exec), value).map_err(|e| e.into_vm_err())
    }
}

/// Print any [`vm::Value`] into a [`vm::String`], calling `toString` methods on objects if present.
///
/// On return, the stack in the provided `exec` is restored to its initial state.
pub fn value_to_string<'gc>(
    ctx: vm::Context<'gc>,
    exec: vm::Execution<'gc, '_>,
    value: vm::Value<'gc>,
) -> Result<vm::String<'gc>, vm::VmError<'gc>> {
    if let vm::Value::String(s) = value {
        Ok(s)
    } else {
        let mut s = String::new();
        pretty_print_value(&mut s, ctx, Some(exec), value).map_err(|e| match e {
            PrintValueError::ToStringError(err) => err,
            PrintValueError::FmtError(_) => unreachable!(),
        })?;
        Ok(ctx.intern(&s))
    }
}

/// Returns a `fmt::Debug` impl that pretty prints any [`vm::Value`].
pub fn debug_value<'gc>(ctx: vm::Context<'gc>, value: vm::Value<'gc>) -> impl fmt::Debug + 'gc {
    struct PrettyValue<'gc> {
        ctx: vm::Context<'gc>,
        value: vm::Value<'gc>,
    }

    impl<'gc> fmt::Debug for PrettyValue<'gc> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            pretty_print_value(f, self.ctx, None, self.value).map_err(|e| match e {
                PrintValueError::ToStringError(_) => unreachable!(),
                PrintValueError::FmtError(e) => e,
            })
        }
    }

    PrettyValue { ctx, value }
}

// Print any value in an expression-like format.
fn pretty_print_value<'gc>(
    f: &mut dyn fmt::Write,
    ctx: vm::Context<'gc>,
    exec: Option<vm::Execution<'gc, '_>>,
    value: vm::Value<'gc>,
) -> Result<(), PrintValueError<'gc>> {
    struct State<'gc, 'a, 'b> {
        writer: &'a mut dyn fmt::Write,
        ctx: vm::Context<'gc>,
        exec: Option<&'a mut vm::Execution<'gc, 'b>>,
        recursive_check: HashSet<*const ()>,
        object_stack: Vec<(vm::String<'gc>, vm::Value<'gc>)>,
        array_stack: Vec<vm::Value<'gc>>,
    }

    fn print_value_inner<'gc, 'a, 'b>(
        state: &mut State<'gc, 'a, 'b>,
        quoted: bool,
        value: vm::Value<'gc>,
    ) -> Result<(), PrintValueError<'gc>> {
        match value {
            vm::Value::String(s) => {
                if quoted {
                    write!(state.writer, "{:?}", s)?;
                } else {
                    write!(state.writer, "{}", s)?;
                }
                Ok(())
            }
            vm::Value::Object(object) => {
                if let Some(exec) = &mut state.exec
                    && let Some(to_string) =
                        object.try_find_field(state.ctx, "toString").ok().flatten()
                {
                    let to_string: vm::Function = vm::FromValue::from_value(state.ctx, to_string)
                        .map_err(vm::VmError::from)?;
                    exec.with_this(object)
                        .call(state.ctx, to_string)
                        .map_err(vm::VmError::from)?;
                    let s: vm::String =
                        exec.stack().consume(state.ctx).map_err(vm::VmError::from)?;

                    if quoted {
                        write!(state.writer, "{:?}", s)?;
                    } else {
                        write!(state.writer, "{}", s)?;
                    }
                    Ok(())
                } else {
                    let obj_ptr = Gc::as_ptr(object.into_inner()) as *const ();
                    if !state.recursive_check.insert(obj_ptr) {
                        write!(state.writer, "<recursive object>")?;
                        return Ok(());
                    }

                    let obj_base = state.object_stack.len();

                    {
                        let Ok(map) = object.try_borrow() else {
                            write!(state.writer, "<borrowed object>")?;
                            return Ok(());
                        };
                        state.object_stack.extend(map.iter());
                    }

                    write!(state.writer, "{{")?;
                    for i in obj_base..state.object_stack.len() {
                        let (key, value) = state.object_stack[i];
                        write!(state.writer, " {}: ", key)?;
                        print_value_inner(state, true, value)?;
                        if i + 1 < state.object_stack.len() {
                            write!(state.writer, ",")?;
                        } else {
                            write!(state.writer, " ")?;
                        }
                    }

                    state.object_stack.truncate(obj_base);
                    state.recursive_check.remove(&obj_ptr);

                    Ok(write!(state.writer, "}}")?)
                }
            }
            vm::Value::Array(array) => {
                let array_ptr = Gc::as_ptr(array.into_inner()) as *const ();
                if !state.recursive_check.insert(array_ptr) {
                    return Ok(write!(state.writer, "<recursive array>")?);
                }

                let arr_base = state.array_stack.len();

                {
                    let Ok(array) = array.try_borrow() else {
                        write!(state.writer, "<borrowed array>")?;
                        return Ok(());
                    };
                    state.array_stack.extend(array.iter());
                }

                write!(state.writer, "[")?;
                for i in arr_base..state.array_stack.len() {
                    print_value_inner(state, true, state.array_stack[i])?;
                    if i + 1 < state.array_stack.len() {
                        write!(state.writer, ", ")?;
                    }
                }

                state.array_stack.truncate(arr_base);
                state.recursive_check.remove(&array_ptr);

                write!(state.writer, "]")?;

                Ok(())
            }
            vm::Value::UserData(user_data) => {
                match user_data.coerce_string(state.ctx) {
                    Some(s) => write!(state.writer, "{:?}", s)?,
                    None => write!(state.writer, "{}", value)?,
                };
                Ok(())
            }
            _ => Ok(write!(state.writer, "{}", value)?),
        }
    }

    if let Some(mut exec) = exec {
        let stack_top = exec.stack().len();
        let r = print_value_inner(
            &mut State {
                writer: f,
                ctx,
                exec: Some(&mut exec.with_stack_bottom(stack_top)),
                recursive_check: HashSet::new(),
                object_stack: Vec::new(),
                array_stack: Vec::new(),
            },
            false,
            value,
        );
        exec.stack().drain(stack_top..);
        r
    } else {
        print_value_inner(
            &mut State {
                writer: f,
                ctx,
                exec: None,
                recursive_check: HashSet::new(),
                object_stack: Vec::new(),
                array_stack: Vec::new(),
            },
            false,
            value,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_format() {
        assert_eq!(
            &split_format("{0}").collect::<Vec<_>>(),
            &[FormatPart::Arg(0)]
        );
        assert_eq!(
            &split_format("{{0}").collect::<Vec<_>>(),
            &[FormatPart::Str("{"), FormatPart::Arg(0)]
        );
        assert_eq!(
            &split_format("{0}{1}foo{2}").collect::<Vec<_>>(),
            &[
                FormatPart::Arg(0),
                FormatPart::Arg(1),
                FormatPart::Str("foo"),
                FormatPart::Arg(2)
            ]
        );
        assert_eq!(
            &split_format("{0}{{1}}foo{2}").collect::<Vec<_>>(),
            &[
                FormatPart::Arg(0),
                FormatPart::Str("{"),
                FormatPart::Arg(1),
                FormatPart::Str("}foo"),
                FormatPart::Arg(2)
            ]
        );
        assert_eq!(
            &split_format("{0{{1}}foo{2").collect::<Vec<_>>(),
            &[
                FormatPart::Str("{0{"),
                FormatPart::Arg(1),
                FormatPart::Str("}foo{2"),
            ]
        );
    }
}
