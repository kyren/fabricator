use std::{
    cell::{Ref, RefCell, RefMut},
    str,
    sync::atomic,
};

use fabricator_vm as vm;
use gc_arena::{Collect, Gc, Rootable};

use crate::{
    string::value_to_string,
    util::{MagicExt as _, Pointer},
};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum BufferType {
    Fixed,
    Growable,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum BufferSeek {
    Start,
    Relative,
    End,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DataType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    F32,
    F64,
    Bool,
    String,
    Text,
}

impl DataType {
    #[inline]
    pub fn fixed_byte_width(self) -> Option<usize> {
        match self {
            DataType::U8 | DataType::I8 | DataType::Bool => Some(1),
            DataType::U16 | DataType::I16 => Some(2),
            DataType::U32 | DataType::I32 | DataType::F32 => Some(4),
            DataType::U64 | DataType::F64 => Some(8),
            DataType::String | DataType::Text => None,
        }
    }
}

pub struct Buffer {
    inner: RefCell<BufferState>,
    numeric_id: i64,
}

struct BufferState {
    buffer_type: BufferType,
    alignment_power_of_2: u32,
    data: Vec<u8>,
    cursor: usize,
}

impl Buffer {
    pub fn new(mut data: Vec<u8>, buffer_type: BufferType, alignment: usize) -> Self {
        static NUMERIC_ID: atomic::AtomicI64 = atomic::AtomicI64::new(0);

        let numeric_id = NUMERIC_ID.fetch_add(1, atomic::Ordering::Relaxed);

        assert!(alignment.is_power_of_two());
        let alignment_power_of_2 = alignment.ilog2();
        let new_len = data.len().checked_next_multiple_of(alignment).unwrap();
        data.resize(new_len, 0);
        Self {
            inner: RefCell::new(BufferState {
                buffer_type,
                alignment_power_of_2,
                data,
                cursor: 0,
            }),
            numeric_id,
        }
    }

    pub fn into_userdata<'gc>(self, ctx: vm::Context<'gc>) -> vm::UserData<'gc> {
        struct BufferMethods;

        impl<'gc> vm::UserDataMethods<'gc> for BufferMethods {
            fn coerce_integer(&self, ud: vm::UserData<'gc>, _ctx: vm::Context<'gc>) -> Option<i64> {
                Some(ud.downcast_static::<Buffer>().unwrap().numeric_id)
            }
        }

        #[derive(Collect)]
        #[collect(no_drop)]
        struct BufferMethodsSingleton<'gc>(Gc<'gc, dyn vm::UserDataMethods<'gc>>);

        impl<'gc> vm::Singleton<'gc> for BufferMethodsSingleton<'gc> {
            fn create(ctx: vm::Context<'gc>) -> Self {
                let methods = ctx.alloc_static(BufferMethods);
                BufferMethodsSingleton(gc_arena::unsize!(methods => dyn vm::UserDataMethods<'gc>))
            }
        }

        let methods = ctx.singleton::<Rootable![BufferMethodsSingleton<'_>]>().0;
        let ud = vm::UserData::new_static(&ctx, self);
        ud.set_methods(&ctx, Some(methods));
        ud
    }

    #[inline]
    pub fn downcast<'gc>(ud: vm::UserData<'gc>) -> Result<&'gc Buffer, vm::BadUserDataType> {
        ud.downcast_static::<Buffer>()
    }

    #[inline]
    pub fn data<'a>(&'a self) -> Ref<'a, [u8]> {
        Ref::map(self.inner.borrow(), |cell| cell.data.as_slice())
    }

    #[inline]
    pub fn data_mut<'a>(&'a self) -> RefMut<'a, [u8]> {
        RefMut::map(self.inner.borrow_mut(), |cell| cell.data.as_mut_slice())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.borrow().data.len()
    }

    #[inline]
    pub fn cursor(&self) -> usize {
        self.inner.borrow().cursor
    }

    pub fn seek<'gc>(&self, seek_type: BufferSeek, seek: isize) {
        let mut buffer = self.inner.borrow_mut();
        let base = match seek_type {
            BufferSeek::Start => 0,
            BufferSeek::Relative => buffer.cursor,
            BufferSeek::End => buffer.data.len(),
        };
        buffer.cursor = base.saturating_add_signed(seek).min(buffer.data.len());
    }
}

impl BufferState {
    fn alignment(&self) -> usize {
        1 << self.alignment_power_of_2
    }

    fn cursor_write(&mut self, data: &[u8]) -> Result<(), vm::RuntimeError> {
        let cursor = self.cursor.next_multiple_of(self.alignment());
        self.write_at(cursor, data)?;
        self.cursor = cursor + data.len();
        Ok(())
    }

    fn cursor_write_unaligned(&mut self, data: &[u8]) -> Result<(), vm::RuntimeError> {
        self.write_at(self.cursor, data)?;
        self.cursor = self.cursor + data.len();
        Ok(())
    }

    fn cursor_read(&mut self, data: &mut [u8]) -> Result<(), vm::RuntimeError> {
        let cursor = self.cursor.next_multiple_of(self.alignment());
        self.read_at(cursor, data)?;
        self.cursor = cursor + data.len();
        Ok(())
    }

    fn cursor_read_until_nul_or_end(&mut self) -> &[u8] {
        let cursor = self.cursor.next_multiple_of(self.alignment());
        let end = self.data[cursor..]
            .iter()
            .position(|&b| b == 0)
            .map(|i| cursor + i)
            .unwrap_or(self.data.len());
        let slice = &self.data[cursor..end];
        self.cursor = end;
        slice
    }

    fn write_at(&mut self, pos: usize, data: &[u8]) -> Result<(), vm::RuntimeError> {
        let write_end = pos + data.len();
        if self.data.len() < write_end {
            match self.buffer_type {
                BufferType::Fixed => {
                    return Err(vm::RuntimeError::msg(format!(
                        "write to pos {write_end} on buffer of length {}",
                        self.data.len()
                    )));
                }
                BufferType::Growable => {
                    let new_len = write_end.next_multiple_of(self.alignment());
                    self.data.resize(new_len, 0);
                }
            }
        }
        self.data[pos..write_end].copy_from_slice(data);
        Ok(())
    }

    fn read_at(&self, pos: usize, data: &mut [u8]) -> Result<(), vm::RuntimeError> {
        let read_end = pos + data.len();
        if self.data.len() < read_end {
            return Err(vm::RuntimeError::msg(format!(
                "read to pos {read_end} on buffer of length {}",
                self.data.len()
            )));
        }
        data.copy_from_slice(&self.data[pos..read_end]);
        Ok(())
    }

    fn read_until_nul_or_end(&mut self, pos: usize) -> &[u8] {
        let end = self.data[pos..]
            .iter()
            .position(|&b| b == 0)
            .map(|i| pos + i)
            .unwrap_or(self.data.len());
        let slice = &self.data[pos..end];
        slice
    }
}

pub fn buffer_create<'gc>(
    ctx: vm::Context<'gc>,
    (size, buf_type, alignment): (usize, vm::UserData<'gc>, usize),
) -> Result<vm::UserData<'gc>, vm::RuntimeError> {
    if !alignment.is_power_of_two() {
        return Err(vm::RuntimeError::msg(format!(
            "buffer alignment {alignment} is not a power of 2"
        )));
    }
    let buf_type = *buf_type.downcast_static::<BufferType>()?;
    Ok(Buffer::new(vec![0; size], buf_type, alignment).into_userdata(ctx))
}

pub fn buffer_write<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::VmError<'gc>> {
    let (buffer, data_type, value): (vm::UserData, vm::UserData, vm::Value) =
        exec.stack().consume(ctx)?;
    let mut buffer = Buffer::downcast(buffer)?.inner.borrow_mut();

    macro_rules! write_value {
        ($val_ty:ty) => {{
            let v: $val_ty = vm::FromValue::from_value(ctx, value)?;
            buffer.cursor_write(&v.to_ne_bytes())?;
        }};
    }
    match *data_type.downcast_static::<DataType>()? {
        DataType::U8 => write_value!(u8),
        DataType::I8 => write_value!(i8),
        DataType::U16 => write_value!(u16),
        DataType::I16 => write_value!(i16),
        DataType::U32 => write_value!(u32),
        DataType::I32 => write_value!(i32),
        DataType::U64 => write_value!(i64),
        DataType::F32 => write_value!(f32),
        DataType::F64 => write_value!(f64),
        DataType::Bool => {
            let b = value.cast_bool();
            buffer.cursor_write(if b { &[1] } else { &[0] })?;
        }
        DataType::String => {
            // `buffer_string` is documented to write a string including a trailing NUL. In order to
            // make sure that writes and reads can be synchronized, we are interpreting that here as
            // writing a string which does *not* contain NUL followed by a single NUL.
            let s: vm::String = value_to_string(ctx, exec.reborrow(), value)?;
            if let Some(end) = s.find('\0') {
                // If the string has an embedded NUL, write the part up to and including the first
                // NUL.
                buffer.cursor_write(s[0..=end].as_bytes())?;
            } else {
                // If the string does not have an embedded NUL, write the whole string followed by
                // a NUL.
                buffer.cursor_write(s.as_bytes())?;
                buffer.cursor_write_unaligned(&[0])?;
            }
        }
        DataType::Text => {
            // `buffer_text` is documented as writing the string without a trailing NUL.
            //
            // We write the *entire* string here and assume that the string byte length is
            // separately stored.
            let s: vm::String = value_to_string(ctx, exec.reborrow(), value)?;
            buffer.cursor_write(s.as_bytes())?;
        }
    }

    Ok(())
}

pub fn buffer_seek<'gc>(
    _ctx: vm::Context<'gc>,
    (buffer, seek_type, seek): (vm::UserData<'gc>, vm::UserData<'gc>, isize),
) -> Result<(), vm::BadUserDataType> {
    let seek_type = *seek_type.downcast_static::<BufferSeek>()?;
    Buffer::downcast(buffer)?.seek(seek_type, seek);
    Ok(())
}

pub fn buffer_read<'gc>(
    ctx: vm::Context<'gc>,
    (buffer, data_type): (vm::UserData<'gc>, vm::UserData<'gc>),
) -> Result<vm::Value<'gc>, vm::RuntimeError> {
    let mut buffer = Buffer::downcast(buffer)?.inner.borrow_mut();
    let data_type = *data_type.downcast_static::<DataType>()?;

    macro_rules! read_value {
        ($val_ty:ty) => {{
            let mut bytes = [0; _];
            buffer.cursor_read(&mut bytes)?;
            vm::IntoValue::into_value(<$val_ty>::from_ne_bytes(bytes), ctx)
        }};
    }
    let v = match data_type {
        DataType::U8 => read_value!(u8),
        DataType::I8 => read_value!(i8),
        DataType::U16 => read_value!(u16),
        DataType::I16 => read_value!(i16),
        DataType::U32 => read_value!(u32),
        DataType::I32 => read_value!(i32),
        DataType::U64 => read_value!(i64),
        DataType::F32 => read_value!(f32),
        DataType::F64 => read_value!(f64),
        DataType::Bool => {
            let mut bytes = [0; 1];
            buffer.cursor_read(&mut bytes)?;
            (bytes[0] != 0).into()
        }
        DataType::String | DataType::Text => {
            // Read the entire rest of the buffer as a string, or until encountering the first
            // NUL character.
            let string = ctx.intern(str::from_utf8(buffer.cursor_read_until_nul_or_end())?);
            string.into()
        }
    };

    Ok(v)
}

pub fn buffer_delete<'gc>(
    _ctx: vm::Context<'gc>,
    buffer: vm::UserData<'gc>,
) -> Result<(), vm::BadUserDataType> {
    let mut buffer = Buffer::downcast(buffer)?.inner.borrow_mut();
    buffer.data = Vec::new();
    buffer.cursor = 0;
    Ok(())
}

pub fn buffer_get_address<'gc>(
    ctx: vm::Context<'gc>,
    buffer: vm::UserData<'gc>,
) -> Result<vm::UserData<'gc>, vm::BadUserDataType> {
    let mut buffer = Buffer::downcast(buffer)?.inner.borrow_mut();
    Ok(Pointer::new(buffer.data.as_mut_ptr()).into_userdata(&ctx))
}

pub fn buffer_get_size<'gc>(
    _ctx: vm::Context<'gc>,
    buffer: vm::UserData<'gc>,
) -> Result<isize, vm::BadUserDataType> {
    let buffer = Buffer::downcast(buffer)?.inner.borrow();
    Ok(buffer.data.len() as isize)
}

pub fn buffer_fill<'gc>(
    ctx: vm::Context<'gc>,
    (buffer, offset, data_type, value, length): (
        vm::UserData<'gc>,
        usize,
        vm::UserData<'gc>,
        vm::Value<'gc>,
        usize,
    ),
) -> Result<(), vm::RuntimeError> {
    let mut buffer = Buffer::downcast(buffer)?.inner.borrow_mut();
    let data_type = *data_type.downcast_static::<DataType>()?;

    let data_width = match data_type.fixed_byte_width() {
        Some(data_width) if data_width.is_multiple_of(buffer.alignment()) => data_width,
        _ => {
            return Err(vm::RuntimeError::msg(format!(
                "data width not a whole multiple of the alignment {}",
                buffer.alignment()
            )));
        }
    };

    if !(length - offset).is_multiple_of(data_width) {
        return Err(vm::RuntimeError::msg(format!(
            "requested write length {} not a whole multiple of data width {}",
            length - offset,
            data_width,
        )));
    }

    let mut pos = offset;
    while pos < length {
        macro_rules! write_value {
            ($val_ty:ty) => {{
                let v: $val_ty = vm::FromValue::from_value(ctx, value)?;
                buffer.write_at(pos, &v.to_ne_bytes())?;
            }};
        }

        match data_type {
            DataType::U8 => write_value!(u8),
            DataType::I8 => write_value!(i8),
            DataType::U16 => write_value!(u16),
            DataType::I16 => write_value!(i16),
            DataType::U32 => write_value!(u32),
            DataType::I32 => write_value!(i32),
            DataType::U64 => write_value!(u64),
            DataType::F32 => write_value!(f32),
            DataType::F64 => write_value!(f64),
            DataType::Bool => {
                let b: bool = vm::FromValue::from_value(ctx, value)?;
                buffer.write_at(pos, if b { &[1] } else { &[0] })?;
            }
            DataType::String | DataType::Text => unreachable!(),
        }

        pos += data_width;
    }

    assert!(pos == length);

    Ok(())
}

pub fn buffer_sizeof<'gc>(
    _ctx: vm::Context<'gc>,
    data_type: vm::UserData<'gc>,
) -> Result<isize, vm::RuntimeError> {
    let data_type = *data_type.downcast_static::<DataType>()?;
    let width = data_type.fixed_byte_width().ok_or_else(|| {
        vm::RuntimeError::msg(format!("{data_type:?} does not have a fixed width"))
    })?;
    Ok(width as isize)
}

pub fn buffer_peek<'gc>(
    ctx: vm::Context<'gc>,
    (buffer, offset, data_type): (vm::UserData, usize, vm::UserData),
) -> Result<vm::Value<'gc>, vm::RuntimeError> {
    let mut buffer = Buffer::downcast(buffer)?.inner.borrow_mut();
    let data_type = *data_type.downcast_static::<DataType>()?;

    macro_rules! read_value {
        ($val_ty:ty) => {{
            let mut bytes = [0; _];
            buffer.read_at(offset, &mut bytes)?;
            vm::IntoValue::into_value(<$val_ty>::from_ne_bytes(bytes), ctx)
        }};
    }
    let v = match data_type {
        DataType::U8 => read_value!(u8),
        DataType::I8 => read_value!(i8),
        DataType::U16 => read_value!(u16),
        DataType::I16 => read_value!(i16),
        DataType::U32 => read_value!(u32),
        DataType::I32 => read_value!(i32),
        DataType::U64 => read_value!(i64),
        DataType::F32 => read_value!(f32),
        DataType::F64 => read_value!(f64),
        DataType::Bool => {
            let mut bytes = [0; 1];
            buffer.read_at(offset, &mut bytes)?;
            (bytes[0] != 0).into()
        }
        DataType::String | DataType::Text => {
            // Read the entire rest of the buffer as a string, or until encountering the first
            // NUL character.
            let string = ctx.intern(str::from_utf8(buffer.read_until_nul_or_end(offset))?);
            string.into()
        }
    };

    Ok(v)
}

pub fn buffer_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    for (name, buffer_type) in [
        ("buffer_fixed", BufferType::Fixed),
        ("buffer_grow", BufferType::Growable),
    ] {
        lib.insert_constant(ctx, name, vm::UserData::new_static(&ctx, buffer_type));
    }

    for (name, buffer_seek) in [
        ("buffer_seek_start", BufferSeek::Start),
        ("buffer_seek_relative", BufferSeek::Relative),
        ("buffer_seek_end", BufferSeek::End),
    ] {
        lib.insert_constant(ctx, name, vm::UserData::new_static(&ctx, buffer_seek));
    }

    for (name, data_type) in [
        ("buffer_u8", DataType::U8),
        ("buffer_i8", DataType::I8),
        ("buffer_u16", DataType::U16),
        ("buffer_i16", DataType::I16),
        ("buffer_u32", DataType::U32),
        ("buffer_i32", DataType::I32),
        ("buffer_u64", DataType::U64),
        ("buffer_f32", DataType::F32),
        ("buffer_f64", DataType::F64),
        ("buffer_bool", DataType::Bool),
        ("buffer_string", DataType::String),
        ("buffer_text", DataType::Text),
    ] {
        lib.insert_constant(ctx, name, vm::UserData::new_static(&ctx, data_type));
    }

    lib.insert_callback(ctx, "buffer_create", buffer_create);
    lib.insert_exec_callback(ctx, "buffer_write", buffer_write);
    lib.insert_callback(ctx, "buffer_seek", buffer_seek);
    lib.insert_callback(ctx, "buffer_read", buffer_read);
    lib.insert_callback(ctx, "buffer_delete", buffer_delete);
    lib.insert_callback(ctx, "buffer_get_address", buffer_get_address);
    lib.insert_callback(ctx, "buffer_get_size", buffer_get_size);
    lib.insert_callback(ctx, "buffer_fill", buffer_fill);
    lib.insert_callback(ctx, "buffer_sizeof", buffer_sizeof);
    lib.insert_callback(ctx, "buffer_peek", buffer_peek);
}
