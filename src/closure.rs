use std::fmt;

use gc_arena::{Collect, Gc};

use crate::{bytecode::ByteCode, constant::Constant, value::String};

#[derive(Collect)]
#[collect(no_drop)]
pub struct Prototype<'gc> {
    pub fixed_params: u8,
    pub constants: Box<[Constant<String<'gc>>]>,
    pub bytecode: ByteCode,
}

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct Closure<'gc>(pub Gc<'gc, Prototype<'gc>>);

impl<'gc> Closure<'gc> {
    pub fn new(proto: Gc<'gc, Prototype<'gc>>) -> Self {
        Closure(proto)
    }
}

impl<'gc> fmt::Debug for Closure<'gc> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_tuple("Function")
            .field(&Gc::as_ptr(self.0))
            .finish()
    }
}

impl<'gc> PartialEq for Closure<'gc> {
    fn eq(&self, other: &Closure<'gc>) -> bool {
        Gc::ptr_eq(self.0, other.0)
    }
}

impl<'gc> Eq for Closure<'gc> {}
