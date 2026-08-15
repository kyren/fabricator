use std::{borrow::Borrow, hash::Hash, ops::ControlFlow, vec};

use fabricator_vm::Span;
use gc_arena::Collect;
use rustc_hash::{FxHashMap, FxHashSet};
use thiserror::Error;

use crate::{
    ast::{self, Walk as _, WalkMut as _},
    constant::Constant,
};

#[derive(Debug, Error)]
pub enum EnumErrorKind {
    #[error("enum name is a duplicate of enum #{0}")]
    DuplicateEnum(usize),
    #[error("enum variant name is a duplicate of a previous one")]
    DuplicateVariantName,
}

#[derive(Debug, Error)]
#[error("{kind}")]
pub struct EnumError {
    pub kind: EnumErrorKind,
    pub span: Span,
}

#[derive(Debug, Error)]
pub enum EnumResolutionErrorKind {
    #[error("variant depends on itself recursively")]
    RecursiveEnumVariant,
    #[error("variant value is not a constant integer")]
    EnumVariantNotConstantInteger,
}

#[derive(Debug, Error)]
#[error("enum #{enum_index} variant #{variant_index} {kind}")]
pub struct EnumResolutionError {
    pub kind: EnumResolutionErrorKind,
    pub enum_index: usize,
    pub variant_index: usize,
}

#[derive(Debug, Error)]
pub enum EnumEvaluationErrorKind {
    #[error("reference to enum with an invalid variant")]
    BadVariant,
    #[error("reference to enum with no variant")]
    NoVariant,
}

#[derive(Debug, Error)]
#[error("{kind}")]
pub struct EnumEvaluationError {
    pub kind: EnumEvaluationErrorKind,
    pub enum_index: usize,
    pub span: Span,
}

pub type EnumValue = i64;

#[derive(Debug, Clone)]
pub enum SourceEnumVariantKind<S> {
    Implicit,
    Expression(ast::Expression<S>),
}

#[derive(Debug, Clone)]
pub struct SourceEnumVariant<S> {
    pub name: ast::Ident<S>,
    pub kind: SourceEnumVariantKind<S>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct SourceEnum<S> {
    pub name: ast::Ident<S>,
    pub variants: Vec<SourceEnumVariant<S>>,
    pub span: Span,
}

/// Gather enum definitions from multiple sources and then later resolve them as a recursively
/// dependent set.
#[derive(Debug, Clone)]
pub struct EnumSetBuilder<S> {
    enums: Vec<SourceEnum<S>>,
    enum_dict: FxHashMap<S, usize>,
    variant_dicts: Vec<FxHashMap<S, usize>>,
}

impl<S> Default for EnumSetBuilder<S> {
    fn default() -> Self {
        Self {
            enums: Vec::new(),
            enum_dict: FxHashMap::default(),
            variant_dicts: Vec::new(),
        }
    }
}

impl<S> EnumSetBuilder<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current count of extracted enums.
    ///
    /// Each enum is assigned a sequential index for identification starting from zero. Checking the
    /// current enum count can be used to determine which enums are extracted from which calls to
    /// [`EnumSetBuilder::extract`].
    pub fn len(&self) -> usize {
        self.enums.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enums.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&SourceEnum<S>> {
        self.enums.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SourceEnum<S>> {
        self.enums.iter()
    }
}

impl<S: Eq + Hash> EnumSetBuilder<S> {
    /// Find a enum index by its name.
    pub fn find<Q: ?Sized>(&self, name: &Q) -> Option<usize>
    where
        S: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.enum_dict.get(name).copied()
    }
}

impl<S: Clone + Eq + Hash> EnumSetBuilder<S> {
    /// Extract all top-level enum statements from a block.
    pub fn extract(&mut self, block: &mut ast::Block<S>) -> Result<(), EnumError> {
        for stmt in &block.statements {
            if let ast::Statement::Enum(enum_stmt) = stmt {
                if let Some(&index) = self.enum_dict.get(&enum_stmt.name.inner) {
                    return Err(EnumError {
                        kind: EnumErrorKind::DuplicateEnum(index),
                        span: enum_stmt.span,
                    });
                }
            }
        }

        let enums = block
            .statements
            .extract_if(.., |s| matches!(s, ast::Statement::Enum(_)));

        for stmt in enums {
            let enum_stmt = match stmt {
                ast::Statement::Enum(enum_stmt) => enum_stmt,
                _ => unreachable!(),
            };

            let mut enum_ = SourceEnum {
                name: enum_stmt.name,
                variants: Vec::new(),
                span: enum_stmt.span,
            };

            let mut variant_dict = FxHashMap::default();

            for (name, value) in enum_stmt.variants {
                if let Some(expr) = value {
                    let span = name.span.combine(expr.span());
                    variant_dict.insert(name.inner.clone(), enum_.variants.len());
                    enum_.variants.push(SourceEnumVariant {
                        name,
                        kind: SourceEnumVariantKind::Expression(expr),
                        span,
                    });
                } else {
                    let span = name.span;
                    variant_dict.insert(name.inner.clone(), enum_.variants.len());
                    enum_.variants.push(SourceEnumVariant {
                        name,
                        kind: SourceEnumVariantKind::Implicit,
                        span,
                    });
                };
            }

            assert!(
                self.enum_dict
                    .insert(enum_.name.inner.clone(), self.enums.len())
                    .is_none()
            );
            self.enums.push(enum_);
            self.variant_dicts.push(variant_dict);
            assert_eq!(self.enums.len(), self.variant_dicts.len());
        }

        Ok(())
    }

    /// Resolve all inter-variant dependencies and return an `EnumSet`.
    ///
    /// Enums and variants will retain the same indexes they had in the `EnumSetBuilder`.
    pub fn resolve(&self) -> Result<EnumSet<S>, EnumResolutionError> {
        // Determine a proper enum variant evaluation order.
        //
        // Add all of the enum variants we need to evaluate to a stack.
        //
        // Take the top entry off of the stack and check to see if it has any un-evaluated
        // dependencies. If it does, then push the popped variant back onto the stack, followed
        // by all of its un-evaluated dependencies. Otherwise if the variant has no un-evaluated
        // dependencies, then it can be evaluated next.
        //
        // We also keep track of in-progress *evaluating* variants (un-evaluated variants that we
        // have encountered before which had an un-evaluated dependency). If we encounter one of
        // these variants more than once without it becoming evaluated in-between, then we know we
        // have a recursive enum variant.

        let mut eval_stack = Vec::new();
        for (enum_index, enum_) in self.enums.iter().enumerate() {
            for var_index in 0..enum_.variants.len() {
                eval_stack.push((enum_index, var_index));
            }
        }

        let mut evaluating_variants = FxHashSet::<(usize, usize)>::default();
        let mut evaluated_variants = FxHashSet::<(usize, usize)>::default();
        let mut eval_order = Vec::<(usize, usize)>::new();
        let mut dependencies = Vec::<(usize, usize)>::new();

        loop {
            let Some((enum_index, variant_index)) = eval_stack.pop() else {
                break;
            };

            if evaluated_variants.contains(&(enum_index, variant_index)) {
                // Variants can be in the evaluation stack more than once since we push every
                // unevaluated variant to the stack first thing to make sure that every one is
                // evaluated. If we encounter an evaluated variant again we can just skip it.
                continue;
            }

            let enum_ = &self.enums[enum_index];
            let variant = &enum_.variants[variant_index];

            dependencies.clear();
            match &variant.kind {
                SourceEnumVariantKind::Implicit => {
                    // Implicit variants depend on the previous enum value, unless they are the
                    // first enum variant, in which case their value is always 0.
                    if variant_index > 0 {
                        dependencies.push((enum_index, variant_index - 1));
                    }
                }
                SourceEnumVariantKind::Expression(expression) => {
                    struct FindEnumVariants<'a, S> {
                        parent: &'a EnumSetBuilder<S>,
                        dependencies: &'a mut Vec<(usize, usize)>,
                    }

                    impl<'a, S> ast::Visitor<S> for FindEnumVariants<'a, S>
                    where
                        S: Eq + Hash,
                    {
                        type Break = ();

                        fn visit_expr(
                            &mut self,
                            expr: &ast::Expression<S>,
                        ) -> ControlFlow<Self::Break> {
                            match expr {
                                ast::Expression::Field(field_expr) => {
                                    if let ast::Expression::Ident(ident) = &*field_expr.base {
                                        if let Some(&ref_enum_index) =
                                            self.parent.enum_dict.get(&ident.inner)
                                        {
                                            if let Some(&ref_variant_index) =
                                                self.parent.variant_dicts[ref_enum_index]
                                                    .get(&field_expr.field.inner)
                                            {
                                                self.dependencies
                                                    .push((ref_enum_index, ref_variant_index));
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }

                            expr.walk(self)
                        }
                    }

                    let _ = ast::Visitor::visit_expr(
                        &mut FindEnumVariants {
                            parent: &self,
                            dependencies: &mut dependencies,
                        },
                        expression,
                    );
                }
            }

            let mut has_unevaluated_dependency = false;
            for (dep_enum_index, dep_variant_index) in dependencies.drain(..) {
                if !evaluated_variants.contains(&(dep_enum_index, dep_variant_index)) {
                    if !has_unevaluated_dependency {
                        has_unevaluated_dependency = true;

                        // We need to evaluate dependencies before this variant, so mark the variant
                        // as in-progress. If we get here a *second* time without the variant
                        // becoming evaluated, then we must have a recursive dependency.
                        if !evaluating_variants.insert((enum_index, variant_index)) {
                            return Err(EnumResolutionError {
                                kind: EnumResolutionErrorKind::RecursiveEnumVariant,
                                enum_index,
                                variant_index,
                            });
                        }

                        // We must push the evaluating variant *before* all of its dependencies.
                        eval_stack.push((enum_index, variant_index));
                    }

                    eval_stack.push((dep_enum_index, dep_variant_index))
                }
            }

            if !has_unevaluated_dependency {
                evaluated_variants.insert((enum_index, variant_index));
                eval_order.push((enum_index, variant_index));
            }
        }

        // Once we have a known-good evaluation order, we can evaluate our interdependent variants
        // in this order and all references should be present.

        let get_evaluated = |evaluated: &FxHashMap<(usize, usize), EnumValue>,
                             enum_index: usize,
                             var_index: usize|
         -> EnumValue {
            match evaluated.get(&(enum_index, var_index)) {
                Some(&v) => v,
                _ => panic!("enum #{enum_index} variant #{var_index} has not been evaluated"),
            }
        };

        let mut evaluated = FxHashMap::default();

        for (enum_index, variant_index) in eval_order {
            match self.enums[enum_index].variants[variant_index].kind.clone() {
                SourceEnumVariantKind::Implicit => {
                    let value = if variant_index == 0 {
                        // The first enum value, if not specified, is always 0.
                        0
                    } else {
                        get_evaluated(&evaluated, enum_index, variant_index - 1).wrapping_add(1)
                    };
                    evaluated.insert((enum_index, variant_index), value);
                }
                SourceEnumVariantKind::Expression(mut expression) => {
                    struct EnumExpander<'a, S>(&'a dyn Fn(&S, &S) -> Option<EnumValue>);

                    impl<'a, S> ast::VisitorMut<S> for EnumExpander<'a, S> {
                        type Break = ();

                        fn visit_expr_mut(
                            &mut self,
                            expr: &mut ast::Expression<S>,
                        ) -> ControlFlow<Self::Break> {
                            match expr {
                                ast::Expression::Field(field_expr) => {
                                    if let ast::Expression::Ident(ident) = &*field_expr.base {
                                        if let Some(val) =
                                            (self.0)(&ident.inner, &field_expr.field.inner)
                                        {
                                            *expr = ast::Expression::Constant(
                                                Constant::Integer(val),
                                                field_expr.span,
                                            );
                                        }
                                    }
                                }
                                _ => {}
                            }

                            expr.walk_mut(self)
                        }
                    }

                    let _ = ast::VisitorMut::visit_expr_mut(
                        &mut EnumExpander(&|enum_name, variant_name| {
                            let &enum_index = self.enum_dict.get(enum_name)?;
                            let &variant_index =
                                self.variant_dicts[enum_index].get(variant_name)?;
                            Some(get_evaluated(&evaluated, enum_index, variant_index))
                        }),
                        &mut expression,
                    );
                    let value = expression
                        .fold_constant()
                        .and_then(|c| c.as_integer())
                        .ok_or(EnumResolutionError {
                            kind: EnumResolutionErrorKind::EnumVariantNotConstantInteger,
                            enum_index,
                            variant_index,
                        })?;
                    evaluated.insert((enum_index, variant_index), value);
                }
            }
        }

        Ok(EnumSet::<S> {
            enums: self
                .enums
                .iter()
                .enumerate()
                .map(|(enum_index, enum_)| Enum {
                    name: enum_.name.inner.clone(),
                    variants: enum_
                        .variants
                        .iter()
                        .enumerate()
                        .map(|(var_index, variant)| EnumVariant {
                            name: variant.name.inner.clone(),
                            value: get_evaluated(&evaluated, enum_index, var_index),
                        })
                        .collect(),
                })
                .collect(),
            enum_dict: self.enum_dict.clone(),
            variant_dicts: self.variant_dicts.clone(),
        })
    }
}

#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct EnumVariant<S> {
    pub name: S,
    pub value: EnumValue,
}

#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct Enum<S> {
    pub name: S,
    pub variants: Vec<EnumVariant<S>>,
}

#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct EnumSet<S> {
    enums: Vec<Enum<S>>,
    enum_dict: FxHashMap<S, usize>,
    variant_dicts: Vec<FxHashMap<S, usize>>,
}

impl<S> Default for EnumSet<S> {
    fn default() -> Self {
        Self {
            enums: Vec::new(),
            enum_dict: FxHashMap::default(),
            variant_dicts: Vec::new(),
        }
    }
}

impl<S> EnumSet<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current count of extracted enums.
    ///
    /// Each enum is assigned a sequential index for identification starting from zero. Checking the
    /// current enum count can be used to determine which enums are extracted from which calls to
    /// [`EnumSetBuilder::extract`].
    pub fn len(&self) -> usize {
        self.enums.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enums.is_empty()
    }

    /// Get an extracted enum.
    pub fn get(&self, index: usize) -> Option<&Enum<S>> {
        self.enums.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Enum<S>> {
        self.enums.iter()
    }
}

impl<S> IntoIterator for EnumSet<S> {
    type Item = Enum<S>;
    type IntoIter = vec::IntoIter<Enum<S>>;

    fn into_iter(self) -> Self::IntoIter {
        self.enums.into_iter()
    }
}

impl<S: Clone + Eq + Hash> FromIterator<Enum<S>> for EnumSet<S> {
    fn from_iter<T: IntoIterator<Item = Enum<S>>>(iter: T) -> Self {
        let mut enums = EnumSet::default();
        enums.merge(iter);
        enums
    }
}

impl<S: Eq + Hash> EnumSet<S> {
    /// Find a enum index by its name.
    pub fn find<Q: ?Sized>(&self, name: &Q) -> Option<usize>
    where
        S: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.enum_dict.get(name).copied()
    }
}

impl<S: Clone + Eq + Hash> EnumSet<S> {
    /// Insert an enum into this set.
    ///
    /// Never overwrites an existing enum. If the given enum shares a name with a pre-existing one,
    /// only the *newly inserted* enum will be returned from [`EnumSet::find`].
    ///
    /// The returned index will always be the previous value of [`EnumSet::len`].
    pub fn insert(&mut self, enum_: Enum<S>) -> usize {
        let mut variant_dict = FxHashMap::default();
        for (i, variant) in enum_.variants.iter().enumerate() {
            variant_dict.insert(variant.name.clone(), i);
        }

        let name = enum_.name.clone();
        let index = self.enums.len();
        self.enums.push(enum_);
        self.variant_dicts.push(variant_dict);
        self.enum_dict.insert(name, index);
        index
    }

    /// Insert all of the given enums into this set.
    pub fn merge(&mut self, enums: impl IntoIterator<Item = Enum<S>>) {
        for enum_ in enums {
            self.insert(enum_);
        }
    }

    /// Expand all enum values referenced anywhere in the given block.
    pub fn expand(&self, block: &mut impl ast::WalkMut<S>) -> Result<(), EnumEvaluationError> {
        struct EnumExpander<'a, S>(&'a EnumSet<S>);

        impl<'a, S: Clone + Eq + Hash> ast::VisitorMut<S> for EnumExpander<'a, S> {
            type Break = EnumEvaluationError;

            fn visit_expr_mut(
                &mut self,
                expr: &mut ast::Expression<S>,
            ) -> ControlFlow<Self::Break> {
                match expr {
                    ast::Expression::Ident(ident) => {
                        if let Some(&enum_index) = self.0.enum_dict.get(ident) {
                            return ControlFlow::Break(EnumEvaluationError {
                                kind: EnumEvaluationErrorKind::NoVariant,
                                enum_index,
                                span: expr.span(),
                            });
                        }
                    }
                    ast::Expression::Field(field_expr) => {
                        if let ast::Expression::Ident(ident) = &*field_expr.base {
                            if let Some(&enum_index) = self.0.enum_dict.get(ident) {
                                if let Some(&var_index) =
                                    self.0.variant_dicts[enum_index].get(&field_expr.field)
                                {
                                    let val = self.0.enums[enum_index].variants[var_index].value;
                                    *expr = ast::Expression::Constant(
                                        Constant::Integer(val),
                                        field_expr.span,
                                    );
                                } else {
                                    return ControlFlow::Break(EnumEvaluationError {
                                        kind: EnumEvaluationErrorKind::BadVariant,
                                        enum_index,
                                        span: expr.span(),
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }

                expr.walk_mut(self)
            }
        }

        if let ControlFlow::Break(err) = block.walk_mut(&mut EnumExpander(self)) {
            Err(err)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    pub use super::*;

    use crate::{lexer::Lexer, parser::ParseSettings, string_interner::StdStringInterner};

    #[test]
    fn test_synthetic() {
        const SOURCE: &str = r#"
            var value = Hello.B;
        "#;

        let enums: EnumSet<_> = [Enum {
            name: "Hello".to_owned(),
            variants: [
                EnumVariant {
                    name: "A".to_owned(),
                    value: 1,
                },
                EnumVariant {
                    name: "B".to_owned(),
                    value: 2,
                },
            ]
            .into_iter()
            .collect(),
        }]
        .into_iter()
        .collect();

        let mut tokens = Vec::new();
        Lexer::tokenize(StdStringInterner, SOURCE, &mut tokens).unwrap();

        let mut block = ParseSettings::strict().parse(tokens).unwrap();

        enums.expand(&mut block).unwrap();

        let ast::Statement::Var(ast::VarDeclarationStmt { vars, .. }) = &block.statements[0] else {
            panic!()
        };

        let var = &vars[0];

        assert!(*var.0 == "value");
        assert!(matches!(
            var.1,
            Some(ast::Expression::Constant(Constant::Integer(2), _))
        ));
    }
}
