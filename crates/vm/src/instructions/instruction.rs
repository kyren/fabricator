use std::fmt;

use gc_arena::Collect;

pub trait IndexType {
    type Index;

    fn index(&self) -> usize;
}

macro_rules! make_idx {
    ($name:ident, $ty:ty, $prefix:literal) => {
        #[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Collect)]
        #[collect(require_static)]
        #[repr(transparent)]
        pub struct $name(pub $ty);

        impl IndexType for $name {
            type Index = $ty;

            #[inline(always)]
            fn index(&self) -> usize {
                self.0 as usize
            }
        }

        impl TryFrom<usize> for $name {
            type Error = <$ty as TryFrom<usize>>::Error;

            #[inline]
            fn try_from(v: usize) -> Result<Self, Self::Error> {
                Ok(Self(<$ty>::try_from(v)?))
            }
        }

        impl fmt::Debug for $name {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl fmt::Display for $name {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl PrettyField for $name {
            #[inline]
            fn fmt(&self, f: &mut dyn fmt::Write) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }
    };
}

make_idx!(RegIdx, u8, "R");
make_idx!(StackIdx, u8, "S");
make_idx!(ConstIdx, u16, "C");
make_idx!(HeapIdx, u16, "H");
make_idx!(ProtoIdx, u16, "P");
make_idx!(MagicIdx, u32, "M");
make_idx!(InstIdx, u32, "I");

macro_rules! for_each_instruction {
    ($macro:ident) => {
        $macro! {
            [basic]
            /// Set the `dest` register to `Value::Undefined`.
            undefined = Undefined { dest: RegIdx };

            [basic]
            /// Set the `dest` register to `Value::Boolean(is_true)`.
            boolean = Boolean { dest: RegIdx, value: bool };

            [basic]
            /// Load a constant into the `dest` register.
            load_constant = LoadConstant { dest: RegIdx, constant: ConstIdx };

            [basic]
            /// Get a heap variable and place it in the `dest` regsiter.
            get_heap = GetHeap { dest: RegIdx, heap: HeapIdx };

            [basic]
            /// Set a heap variable from the `source` register.
            set_heap = SetHeap { heap: HeapIdx, source: RegIdx };

            [basic]
            /// Reset an *owned* heap variable.
            ///
            /// This resets a heap variable to the value `Undefined`, and also  disconnects heap
            /// variables that are shared with any previously created closures.
            reset_heap = ResetHeap { heap: HeapIdx };

            [basic] globals = Globals { dest: RegIdx };

            [basic]
            /// Push the top value of the `self` stack onto the `self` stack.
            push_this = PushThis {};

            [basic]
            /// Pop the top value off of the `self` stack.
            pop_this = PopThis {};

            [basic]
            /// Get the value at the top of the `self` stack.
            this = This { dest: RegIdx };

            [basic]
            /// Set the value at the top of the `self` stack.
            set_this = SetThis { source: RegIdx };

            [basic]
            /// Get the value one under the top of the `self` stack.
            other = Other { dest: RegIdx };

            [basic] closure = Closure { dest: RegIdx, proto: ProtoIdx, bind_this: bool };

            [basic]
            /// Set the `dest` register to the currently executing closure.
            current_closure = CurrentClosure { dest: RegIdx };

            [basic] arg_count = ArgCount { dest: RegIdx };

            [basic]
            /// Get the argument at the given index and place it in the `dest` register.
            ///
            /// If the argument index is out of range of the current argument list, then the
            /// destination register is set to `Undefined`.
            arg_get = ArgGet { dest: RegIdx, index: StackIdx };

            [basic]
            /// Get the argment at the index pointed to by the `index` register and place it in the
            /// `dest` register.
            ///
            /// If the argument index is out of range of the current argument list, then the
            /// destination register is set to `Undefined`.
            arg_get_at = ArgGetAt { dest: RegIdx, index: RegIdx };

            [basic] new_object = NewObject { dest: RegIdx };
            [basic] new_array = NewArray { dest: RegIdx };

            [basic] get_field = GetField { dest: RegIdx, target: RegIdx, key: RegIdx };
            [basic] set_field = SetField  { target: RegIdx, key: RegIdx, value: RegIdx };
            [basic] get_field_const = GetFieldConst { dest: RegIdx, target: RegIdx, key: ConstIdx };
            [basic] set_field_const = SetFieldConst  { target: RegIdx, key: ConstIdx, value: RegIdx };

            [basic] get_index = GetIndex { dest: RegIdx, target: RegIdx, index: RegIdx };
            [basic] set_index = SetIndex  { target: RegIdx, index: RegIdx, value: RegIdx };
            [basic] get_index_const = GetIndexConst { dest: RegIdx, target: RegIdx, index: ConstIdx };
            [basic] set_index_const = SetIndexConst { target: RegIdx, index: ConstIdx, value: RegIdx };

            [basic] copy = Copy { dest: RegIdx, source: RegIdx };

            [basic] is_defined = IsDefined { dest: RegIdx, arg: RegIdx };
            [basic] is_undefined = IsUndefined { dest: RegIdx, arg: RegIdx };
            [basic] test = Test { dest: RegIdx, arg: RegIdx };
            [basic] not = Not { dest: RegIdx, arg: RegIdx };

            [basic] negate = Negate { dest: RegIdx, arg: RegIdx };
            [basic] bit_negate = BitNegate { dest: RegIdx, arg: RegIdx };
            [basic] increment = Increment { dest: RegIdx, arg: RegIdx };
            [basic] decrement = Decrement { dest: RegIdx, arg: RegIdx };

            [basic] add = Add { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] subtract = Subtract { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] multiply = Multiply { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] divide = Divide { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] remainder = Remainder { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] int_divide = IntDivide { dest: RegIdx, left: RegIdx, right: RegIdx };

            [basic] is_equal = IsEqual { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] is_not_equal = IsNotEqual { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] is_less = IsLess { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] is_less_equal = IsLessEqual { dest: RegIdx, left: RegIdx, right: RegIdx };

            [basic] and = And { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] or = Or { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] xor = Xor { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] bit_and = BitAnd { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] bit_or = BitOr { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] bit_xor = BitXor { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] bit_shift_left = BitShiftLeft { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] bit_shift_right = BitShiftRight { dest: RegIdx, left: RegIdx, right: RegIdx };
            [basic] null_coalesce = NullCoalesce { dest: RegIdx, left: RegIdx, right: RegIdx };

            [basic]
            /// Push a new frame on the stack.
            push_stack_frame = PushStackFrame {};

            [basic]
            /// Pops the topmost stack frame.
            pop_stack_frame = PopStackFrame {};

            [basic]
            /// Merge the topmost stack frame with the one below it.
            join_stack_frame = JoinStackFrame {};

            [basic]
            /// Split the topmost stack frame into a new frame starting at `base`.
            split_stack_frame = SplitStackFrame { base: StackIdx };

            [basic]
            /// Push a value onto the top of the current stack frame.
            stack_push = StackPush { source: RegIdx };

            [basic]
            /// Push two values onto the top of the topmost stack frame.
            stack_push_2 = StackPush2 { source_a: RegIdx, source_b: RegIdx };

            [basic]
            /// Push three values onto the top of the topmost stack frame.
            stack_push_3 = StackPush3 { source_a: RegIdx, source_b: RegIdx, source_c: RegIdx };

            [basic]
            /// Push four values onto the top of the topmost stack frame.
            stack_push_4 = StackPush4 {
                source_a: RegIdx,
                source_b: RegIdx,
                source_c: RegIdx,
                source_d: RegIdx,
            };

            [basic]
            /// Push all arguments starting at the given argument to the current stack frame.
            stack_push_args = StackPushArgs { first_index: StackIdx };

            [basic]
            /// Get the element of the current stack frame at the given index and place it in the
            /// `dest` register.
            ///
            /// If the stack index is out of range of the current stack frame, then the destination
            /// register is set to `Undefined`.
            stack_get = StackGet { dest: RegIdx, index: StackIdx };

            [basic] get_magic = GetMagic { dest: RegIdx, magic: MagicIdx };
            [basic] set_magic = SetMagic { magic: MagicIdx, source: RegIdx };

            [basic]
            /// Throw an error located at the given register.
            throw = Throw { source: RegIdx };

            [jump] jump = Jump { target: InstIdx };

            [jump_if]
            jump_if = JumpIf {
                target: InstIdx,
                arg: RegIdx,
                is_true: bool,
            };

            [jump_if]
            jump_if_undefined = JumpIfUndefined {
                target: InstIdx,
                arg: RegIdx,
                is_undefined: bool,
            };

            [jump_if]
            jump_if_equal = JumpIfEqual {
                target: InstIdx,
                left: RegIdx,
                right: RegIdx,
            };

            [jump_if]
            jump_if_not_equal = JumpIfNotEqual {
                target: InstIdx,
                left: RegIdx,
                right: RegIdx,
            };

            [jump_if]
            jump_if_less = JumpIfLess {
                target: InstIdx,
                left: RegIdx,
                right: RegIdx,
            };

            [jump_if]
            jump_if_less_equal = JumpIfLessEqual {
                target: InstIdx,
                left: RegIdx,
                right: RegIdx,
            };

            [control]
            /// Call a function with arguments in the topmost stack frame.
            ///
            /// Pops all arguments from the topmost stack frame, then pushes all returns as a new
            /// stack frame.
            ///
            /// If a `self` object is given then the the provided `self` will be automatically
            /// pushed before calling and popped after returning, with one exception: If the
            /// `func` object is a function with its own bound `self` value, the provided `self`
            /// object will be *ignored*. This is the only way to perform this operation without
            /// potentially pushing two `self` values.
            call = Call { func: RegIdx, this: Option<RegIdx> };

            [control]
            /// Return with values in the topmost stack frame.
            return_ = Return {};
        }
    };
}

macro_rules! define_instruction {
    ($(
        [$_category:ident] $(#[$attr:meta])* $snake_name:ident = $name:ident { $($field:ident: $field_ty:ty),* $(,)? };
    )*) => {
        #[derive(Copy, Clone, Eq, PartialEq)]
        pub enum Instruction {
            $(
                $(#[$attr])*
                $name {
                    $($field: $field_ty),*
                }
            ),*
        }
    };
}
for_each_instruction!(define_instruction);

trait PrettyField {
    fn fmt(&self, f: &mut dyn fmt::Write) -> fmt::Result;
}

impl PrettyField for bool {
    fn fmt(&self, f: &mut dyn fmt::Write) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl<T: PrettyField> PrettyField for Option<T> {
    fn fmt(&self, f: &mut dyn fmt::Write) -> fmt::Result {
        match self {
            Some(v) => v.fmt(f),
            None => write!(f, "_"),
        }
    }
}

impl Instruction {
    pub fn pretty_print(self, f: &mut dyn fmt::Write) -> fmt::Result {
        macro_rules! impl_debug {
            ($(
                [$_category:ident] $(#[$_attr:meta])* $snake_name:ident = $name:ident { $($field:ident: $field_ty:ty),* $(,)? };
            )*) => {
                match self {
                    $(Instruction::$name { $($field),* } => {
                        write!(f, stringify!($snake_name))?;
                        write!(f, "(")?;
                        #[allow(unused, unused_mut)]
                        let mut prev = false;
                        $(
                            if prev {
                                write!(f, ", ")?;
                            }
                            #[allow(unused)]
                            {
                                prev = true;
                            }

                            write!(f, stringify!($field))?;
                            write!(f, "=")?;
                            PrettyField::fmt(&$field, f)?;
                        )*
                        write!(f, ")")?;
                    }),*
                }
            };
        }

        for_each_instruction!(impl_debug);
        Ok(())
    }
}

impl fmt::Debug for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.pretty_print(f)
    }
}
