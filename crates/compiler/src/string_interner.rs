use fabricator_vm as vm;

pub trait StringInterner {
    type String;

    fn intern(&mut self, s: &str) -> Self::String;

    fn intern_static(&mut self, s: &'static str) -> Self::String {
        self.intern(s)
    }
}

pub struct VmInterner<'gc>(vm::Context<'gc>);

impl<'gc> VmInterner<'gc> {
    pub fn new(ctx: vm::Context<'gc>) -> Self {
        Self(ctx)
    }
}

impl<'gc> StringInterner for VmInterner<'gc> {
    type String = vm::String<'gc>;

    fn intern(&mut self, s: &str) -> vm::String<'gc> {
        self.0.intern(s)
    }

    fn intern_static(&mut self, s: &'static str) -> Self::String {
        self.0.intern_static(s)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct StdStringInterner;

impl StringInterner for StdStringInterner {
    type String = String;

    fn intern(&mut self, s: &str) -> Self::String {
        s.to_owned()
    }
}
