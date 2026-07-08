use fabricator_util::typed_id_map::SecondaryMap;
use rustc_hash::FxHashMap;

use crate::{constant::Constant, graph::dfs::topological_order, ir};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum InstructionOutputType {
    Undefined,
    Scalar,
    String,
    Object,
    Array,
    Function,
    Any,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StateEffect {
    None,
    Read,
    Write,
}

impl StateEffect {
    pub fn can_write(self) -> bool {
        match self {
            StateEffect::None | StateEffect::Read => false,
            StateEffect::Write => true,
        }
    }

    pub fn has_dependency(self, other: Self) -> bool {
        match (self, other) {
            (Self::Read, Self::Write) | (Self::Write, Self::Read) | (Self::Write, Self::Write) => {
                true
            }
            _ => false,
        }
    }

    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            (_, Self::Write) | (Self::Write, _) => Self::Write,
            (_, Self::Read) | (Self::Read, _) => Self::Read,
            (Self::None, Self::None) => Self::None,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum VariableEffect<V> {
    None,
    Read(V),
    Write(V),
    ReadAny,
    WriteAny,
}

impl<V> VariableEffect<V> {
    pub fn can_write(self) -> bool {
        match self {
            VariableEffect::Write(_) | VariableEffect::WriteAny => true,
            _ => false,
        }
    }
}

impl<V: Eq> VariableEffect<V> {
    pub fn has_dependency(self, other: Self) -> bool {
        match (self, other) {
            (Self::None, _) | (_, Self::None) => false,
            (Self::Read(_) | Self::ReadAny, Self::Read(_) | Self::ReadAny) => false,
            (Self::Read(self_var_id), Self::Write(other_var_id))
            | (Self::Write(self_var_id), Self::Read(other_var_id))
            | (Self::Write(self_var_id), Self::Write(other_var_id)) => self_var_id == other_var_id,
            (Self::Read(_), Self::WriteAny) => true,
            (Self::Write(_), Self::ReadAny | Self::WriteAny) => true,
            (Self::ReadAny, Self::Write(_) | Self::WriteAny) => true,
            (Self::WriteAny, Self::Read(_) | Self::Write(_) | Self::ReadAny | Self::WriteAny) => {
                true
            }
        }
    }

    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            (VariableEffect::None, other) => other,
            (this, VariableEffect::None) => this,

            (VariableEffect::WriteAny, _)
            | (_, VariableEffect::WriteAny)
            | (VariableEffect::ReadAny, VariableEffect::Write(_))
            | (VariableEffect::Write(_), VariableEffect::ReadAny) => VariableEffect::WriteAny,

            (VariableEffect::Read(self_var_id), VariableEffect::Write(other_var_id))
            | (VariableEffect::Write(self_var_id), VariableEffect::Read(other_var_id))
            | (VariableEffect::Write(self_var_id), VariableEffect::Write(other_var_id)) => {
                if self_var_id == other_var_id {
                    VariableEffect::Write(self_var_id)
                } else {
                    VariableEffect::WriteAny
                }
            }

            (VariableEffect::Read(_), VariableEffect::ReadAny)
            | (VariableEffect::ReadAny, VariableEffect::Read(_))
            | (VariableEffect::ReadAny, VariableEffect::ReadAny) => VariableEffect::ReadAny,

            (VariableEffect::Read(self_var_id), VariableEffect::Read(other_var_id)) => {
                if self_var_id == other_var_id {
                    VariableEffect::Read(self_var_id)
                } else {
                    VariableEffect::ReadAny
                }
            }
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct InstructionEffects {
    pub variable: VariableEffect<ir::VarId>,
    pub shadow: VariableEffect<ir::ShadowVar>,
    pub global: StateEffect,
    pub can_error: bool,
}

impl InstructionEffects {
    pub fn none() -> Self {
        Self {
            variable: VariableEffect::None,
            shadow: VariableEffect::None,
            global: StateEffect::None,
            can_error: false,
        }
    }

    /// Instruction can be a variable, shadow, or global state write.
    pub fn can_write(&self) -> bool {
        self.variable.can_write() || self.shadow.can_write() || self.global.can_write()
    }

    /// Instruction can have a write effect or can error.
    pub fn has_effect(&self) -> bool {
        self.can_write() || self.can_error
    }

    /// Reordering an instruction with these effects and an instruction with `other` effects can
    /// change the observable properties of a program.
    pub fn has_dependency(&self, other: &Self) -> bool {
        self.variable.has_dependency(other.variable)
            || self.shadow.has_dependency(other.shadow)
            || self.global.has_dependency(other.global)
            || (self.can_error && other.can_error)
            || (self.can_error && other.can_write())
            || (self.can_write() && other.can_error)
    }

    pub fn combine(&self, other: &Self) -> Self {
        Self {
            variable: self.variable.combine(other.variable),
            shadow: self.shadow.combine(other.shadow),
            global: self.global.combine(other.global),
            can_error: self.can_error || other.can_error,
        }
    }
}

#[derive(Debug)]
pub struct InstructionTypeAndEffects {
    pub output_type: Option<InstructionOutputType>,
    pub effects: InstructionEffects,
}

#[derive(Debug)]
pub struct BranchEffects {
    pub can_error: bool,
}

impl BranchEffects {
    pub fn has_effect(&self) -> bool {
        self.can_error
    }
}

#[derive(Debug)]
pub struct TypesAndEffects {
    pub instructions: SecondaryMap<ir::InstId, InstructionTypeAndEffects>,
    pub branches: SecondaryMap<ir::BlockId, BranchEffects>,
}

impl TypesAndEffects {
    /// Produce a `TypeAndEffects` struct, containing type and effect information for all
    /// *reachable* instructions and branches.
    pub fn analyze<S>(ir: &ir::Function<S>) -> Self {
        let mut this = Self {
            instructions: Default::default(),
            branches: Default::default(),
        };

        let reachable_blocks =
            topological_order(ir.start_block, |b| ir.blocks[b].exit.kind.successors());

        let mut shadow_upsilon_sources: FxHashMap<ir::ShadowVar, Vec<ir::InstId>> =
            FxHashMap::default();

        // Iterate in topological order so we always encounter instructions before their uses.
        for &block_id in &reachable_blocks {
            let block = &ir.blocks[block_id];

            for &inst_id in &block.instructions {
                let inst = &ir.instructions[inst_id];
                let type_and_effects = match inst.kind {
                    ir::InstructionKind::NoOp => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::Copy(source) => {
                        let source_type = this.instructions[source].output_type;
                        InstructionTypeAndEffects {
                            output_type: source_type,
                            effects: InstructionEffects::none(),
                        }
                    }
                    ir::InstructionKind::Constant(ref constant) => InstructionTypeAndEffects {
                        output_type: Some(match constant {
                            Constant::Undefined => InstructionOutputType::Undefined,
                            Constant::Boolean(_) | Constant::Integer(_) | Constant::Float(_) => {
                                InstructionOutputType::Scalar
                            }
                            Constant::String(_) => InstructionOutputType::String,
                        }),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::Closure { .. } => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Function),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::OpenVariable(var_id)
                    | ir::InstructionKind::SetVariable(var_id, _)
                    | ir::InstructionKind::CloseVariable(var_id) => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects {
                            variable: VariableEffect::Write(var_id),
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::GetVariable(_) => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Any),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::GetMagic(_) => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Any),
                        effects: InstructionEffects {
                            global: StateEffect::Read,
                            can_error: true,
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::SetMagic(_, _) => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects {
                            global: StateEffect::Write,
                            can_error: true,
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::Globals => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Object),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::This => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Any),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::Other => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Any),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::CurrentClosure => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Function),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::OpenThisScope(_)
                    | ir::InstructionKind::SetThis(_, _)
                    | ir::InstructionKind::CloseThisScope(_) => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects {
                            global: StateEffect::Write,
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::NewObject => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Object),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::NewArray => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Array),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::FixedArgument(_) => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Any),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::ArgumentCount => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Scalar),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::Argument(_) => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Any),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::GetField { .. }
                    | ir::InstructionKind::GetFieldConst { .. }
                    | ir::InstructionKind::GetIndex { .. }
                    | ir::InstructionKind::GetIndexConst { .. } => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Any),
                        effects: InstructionEffects {
                            global: StateEffect::Read,
                            can_error: true,
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::SetField { .. }
                    | ir::InstructionKind::SetFieldConst { .. }
                    | ir::InstructionKind::SetIndex { .. }
                    | ir::InstructionKind::SetIndexConst { .. } => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects {
                            global: StateEffect::Write,
                            can_error: true,
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::Phi(shadow) => {
                        let mut output_type = None;
                        for &source_inst in &shadow_upsilon_sources[&shadow] {
                            let source_type = this.instructions[source_inst].output_type.unwrap();

                            output_type = Some(if let Some(output_type) = output_type {
                                if output_type == source_type {
                                    output_type
                                } else {
                                    InstructionOutputType::Any
                                }
                            } else {
                                source_type
                            });
                        }

                        InstructionTypeAndEffects {
                            output_type: Some(output_type.unwrap()),
                            effects: InstructionEffects {
                                shadow: VariableEffect::Read(shadow),
                                ..InstructionEffects::none()
                            },
                        }
                    }
                    ir::InstructionKind::Upsilon(shadow, source_inst) => {
                        shadow_upsilon_sources
                            .entry(shadow)
                            .or_default()
                            .push(source_inst);
                        InstructionTypeAndEffects {
                            output_type: None,
                            effects: InstructionEffects {
                                shadow: VariableEffect::Write(shadow),
                                ..InstructionEffects::none()
                            },
                        }
                    }
                    ir::InstructionKind::UnOp { op, source } => {
                        let source_type = this.instructions[source].output_type.unwrap();
                        let can_error = match op {
                            ir::UnOp::Negate
                            | ir::UnOp::BitNegate
                            | ir::UnOp::Increment
                            | ir::UnOp::Decrement => source_type != InstructionOutputType::Scalar,
                            ir::UnOp::IsDefined
                            | ir::UnOp::IsUndefined
                            | ir::UnOp::Test
                            | ir::UnOp::Not => false,
                        };

                        InstructionTypeAndEffects {
                            output_type: Some(InstructionOutputType::Scalar),
                            effects: InstructionEffects {
                                can_error,
                                ..InstructionEffects::none()
                            },
                        }
                    }
                    ir::InstructionKind::BinOp { left, op, right } => {
                        let left_type = this.instructions[left].output_type.unwrap();
                        let right_type = this.instructions[right].output_type.unwrap();
                        let (output_type, can_error) = match op {
                            ir::BinOp::NullCoalesce => {
                                if left_type == InstructionOutputType::Undefined {
                                    // inst.set_kind(ir::InstructionKind::Copy(right));
                                    (right_type, false)
                                } else if left_type != InstructionOutputType::Any {
                                    // inst.set_kind(ir::InstructionKind::Copy(left));
                                    (left_type, false)
                                } else {
                                    (InstructionOutputType::Any, false)
                                }
                            }
                            ir::BinOp::Add
                            | ir::BinOp::Sub
                            | ir::BinOp::Mult
                            | ir::BinOp::Div
                            | ir::BinOp::Rem
                            | ir::BinOp::IDiv
                            | ir::BinOp::LessThan
                            | ir::BinOp::LessEqual
                            | ir::BinOp::GreaterThan
                            | ir::BinOp::GreaterEqual
                            | ir::BinOp::BitAnd
                            | ir::BinOp::BitOr
                            | ir::BinOp::BitXor
                            | ir::BinOp::BitShiftLeft
                            | ir::BinOp::BitShiftRight => (
                                InstructionOutputType::Scalar,
                                left_type != InstructionOutputType::Scalar
                                    || right_type != InstructionOutputType::Scalar,
                            ),
                            ir::BinOp::Equal
                            | ir::BinOp::NotEqual
                            | ir::BinOp::And
                            | ir::BinOp::Or
                            | ir::BinOp::Xor => (InstructionOutputType::Scalar, false),
                        };

                        InstructionTypeAndEffects {
                            output_type: Some(output_type),
                            effects: InstructionEffects {
                                can_error,
                                ..InstructionEffects::none()
                            },
                        }
                    }
                    ir::InstructionKind::OpenCallScope { .. } => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects {
                            global: StateEffect::Write,
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::PushStack(_, _) => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects {
                            global: StateEffect::Write,
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::Call { .. } => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects {
                            variable: VariableEffect::WriteAny,
                            global: StateEffect::Write,
                            can_error: true,
                            ..InstructionEffects::none()
                        },
                    },
                    ir::InstructionKind::GetStack(_, _) => InstructionTypeAndEffects {
                        output_type: Some(InstructionOutputType::Any),
                        effects: InstructionEffects::none(),
                    },
                    ir::InstructionKind::CloseCallScope(_) => InstructionTypeAndEffects {
                        output_type: None,
                        effects: InstructionEffects {
                            global: StateEffect::Write,
                            ..InstructionEffects::none()
                        },
                    },
                };

                assert_eq!(
                    type_and_effects.output_type.is_some(),
                    inst.kind.has_output()
                );

                this.instructions.insert(inst_id, type_and_effects);
            }

            if let ir::ExitKind::Branch { cond, .. } = block.exit.kind {
                let branch_can_error = match cond {
                    ir::BranchCondition::IsDefined(_)
                    | ir::BranchCondition::IsUndefined(_)
                    | ir::BranchCondition::IsTrue(_)
                    | ir::BranchCondition::IsFalse(_)
                    | ir::BranchCondition::Equal(_, _)
                    | ir::BranchCondition::NotEqual(_, _) => false,
                    ir::BranchCondition::LessThan(a, b)
                    | ir::BranchCondition::LessEqual(a, b)
                    | ir::BranchCondition::GreaterThan(a, b)
                    | ir::BranchCondition::GreaterEqual(a, b) => {
                        this.instructions[a].output_type.unwrap() != InstructionOutputType::Scalar
                            || this.instructions[b].output_type.unwrap()
                                != InstructionOutputType::Scalar
                    }
                };

                this.branches.insert(
                    block_id,
                    BranchEffects {
                        can_error: branch_can_error,
                    },
                );
            }
        }

        this
    }
}
