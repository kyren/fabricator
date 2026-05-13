use std::{
    borrow::Borrow,
    collections::{HashMap, hash_map},
    hash::Hash,
};

use fabricator_util::index_containers::{IndexMap, IndexSet};
use fabricator_vm::Span;
use gc_arena::Collect;
use thiserror::Error;

use crate::tokens::{Token, TokenKind};

#[derive(Debug, Error)]
pub enum MacroErrorKind {
    #[error("`#macro` must be at the beginning of a line")]
    TrailingMacro,
    #[error("bad or missing macro name, must be an identifier")]
    BadMacroName,
    #[error("macro name is a duplicate of macro #{0}")]
    DuplicateMacro(usize),
}

#[derive(Debug, Error)]
#[error("{kind}")]
pub struct MacroError {
    pub kind: MacroErrorKind,
    pub span: Span,
}

#[derive(Debug, Error)]
#[error("macro #{0} depends on itself recursively")]
pub struct RecursiveMacro(pub usize);

pub type SyntheticMacros<S> = HashMap<S, Vec<Token<S>>>;

#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct Macro<S> {
    pub name: S,
    pub config: Option<S>,
    pub span: Span,
    pub tokens: Vec<Token<S>>,
}

/// The set of macros with a specific name, mapping from configuration to macro index for that
/// configuration.
#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct ConfigurationSet<S> {
    /// Macro specified with no configuration option.
    pub default: Option<usize>,

    /// Macros specified with named configurations.
    pub for_config: HashMap<S, usize>,
}

impl<S> Default for ConfigurationSet<S> {
    fn default() -> Self {
        Self {
            default: None,
            for_config: HashMap::new(),
        }
    }
}

impl<S: Eq + Hash> ConfigurationSet<S> {
    pub fn index_for_config<Q: ?Sized>(&self, config: &Q) -> Option<usize>
    where
        S: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.for_config.get(config).copied().or(self.default)
    }
}

/// Gather a set of macro definitions from multiple sources and then later resolve them as a
/// potentially recursively dependent set.
#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct MacroSet<S> {
    macros: Vec<Macro<S>>,
    macro_dict: HashMap<S, ConfigurationSet<S>>,
}

impl<S> Default for MacroSet<S> {
    fn default() -> Self {
        Self {
            macros: Vec::new(),
            macro_dict: HashMap::new(),
        }
    }
}

impl<S> MacroSet<S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.macros.is_empty()
    }

    /// The current count of extracted macros.
    ///
    /// Each macro is assigned a sequential index for identification starting from zero. Checking
    /// the current macro count can be used to determine which macros are extracted from which calls
    /// to [`MacroSet::extract`].
    pub fn len(&self) -> usize {
        self.macros.len()
    }

    /// Get an extracted macro.
    pub fn get(&self, index: usize) -> Option<&Macro<S>> {
        self.macros.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Macro<S>> {
        self.macros.iter()
    }
}

impl<S: Eq + Hash> MacroSet<S> {
    /// Find the configuration set for a macro by name.
    pub fn find<Q: ?Sized>(&self, name: &Q) -> Option<&ConfigurationSet<S>>
    where
        S: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.macro_dict.get(name)
    }

    /// Find a macro index for a specific configuration by macro name.
    pub fn find_config<Q1: ?Sized, Q2: ?Sized>(&self, config: &Q1, name: &Q2) -> Option<usize>
    where
        S: Borrow<Q1> + Borrow<Q2>,
        Q1: Hash + Eq,
        Q2: Hash + Eq,
    {
        self.macro_dict
            .get(name)
            .and_then(|c| c.for_config.get(config).copied().or(c.default))
    }
}

impl<S: Clone + Eq + Hash> MacroSet<S> {
    /// Extract macros from the given token list and store them.
    ///
    /// This wiill extract all `#macro NAME <TOKENS>` directives from the token list. Macros must
    /// be the first token following a newline, and the macro TOKENS list is interpreted up to
    /// the following newline or eof. All of the tokens that make up the macro are removed, not
    /// including the trailing newline or eof.
    ///
    /// Macros can also be of the form `#macro CONFIG:NAME <TOKENS>`, which marks the macro as
    /// applying only under a specific, named configuration, which can be set during expansion.
    /// These macros will override any default if the named configuration identifier matches the
    /// selected configuration.
    ///
    /// If an error is encountered, the provided token buffer will be in an unspecified state.
    pub fn extract(&mut self, tokens: &mut Vec<Token<S>>) -> Result<(), MacroError> {
        let mut token_iter = tokens.drain(..).peekable();
        let mut filtered_tokens = Vec::new();
        let mut prev_token_was_newline = true;

        while let Some(token) = token_iter.next() {
            if matches!(token.kind, TokenKind::Macro) {
                if !prev_token_was_newline {
                    return Err(MacroError {
                        kind: MacroErrorKind::TrailingMacro,
                        span: token.span,
                    });
                }

                let mut macro_span = token.span;

                let t = token_iter.next();
                let Some(Token {
                    kind: TokenKind::Identifier(config_or_name),
                    span: config_or_name_span,
                }) = t
                else {
                    return Err(MacroError {
                        kind: MacroErrorKind::BadMacroName,
                        span: t.map(|t| t.span).unwrap_or(Span::null()),
                    });
                };

                let config;
                let macro_name;

                // An identifier followed by a colon is a configuration-specific macro, with the
                // config name before the colon.
                if let Some(Token {
                    kind: TokenKind::Colon,
                    ..
                }) = token_iter.peek()
                {
                    config = Some(config_or_name);
                    token_iter.next();

                    let t = token_iter.next();
                    let Some(Token {
                        kind: TokenKind::Identifier(name),
                        span: name_span,
                    }) = t
                    else {
                        return Err(MacroError {
                            kind: MacroErrorKind::BadMacroName,
                            span: t.map(|t| t.span).unwrap_or(Span::null()),
                        });
                    };

                    macro_name = name;
                    macro_span = macro_span.combine(name_span);
                } else {
                    config = None;
                    macro_name = config_or_name;
                    macro_span = macro_span.combine(config_or_name_span);
                }

                let mut macro_tokens = Vec::new();
                while let Some(t) = token_iter.peek() {
                    if matches!(t.kind, TokenKind::EndOfStream | TokenKind::Newline) {
                        break;
                    } else {
                        let token = token_iter.next().unwrap();
                        macro_span = macro_span.combine(token.span);
                        macro_tokens.push(token);
                    }
                }

                let named = self.macro_dict.entry(macro_name.clone()).or_default();

                if let Some(config) = config.clone() {
                    match named.for_config.entry(config) {
                        hash_map::Entry::Occupied(occupied) => {
                            return Err(MacroError {
                                kind: MacroErrorKind::DuplicateMacro(*occupied.get()),
                                span: macro_span,
                            });
                        }
                        hash_map::Entry::Vacant(vacant) => {
                            vacant.insert(self.macros.len());
                        }
                    }
                } else {
                    if let Some(def) = named.default {
                        return Err(MacroError {
                            kind: MacroErrorKind::DuplicateMacro(def),
                            span: macro_span,
                        });
                    }
                    named.default = Some(self.macros.len());
                }

                self.macros.push(Macro {
                    name: macro_name,
                    config,
                    span: macro_span,
                    tokens: macro_tokens,
                });
            } else {
                prev_token_was_newline = matches!(token.kind, TokenKind::Newline);
                filtered_tokens.push(token);
            }
        }

        drop(token_iter);
        *tokens = filtered_tokens;

        Ok(())
    }

    /// Apply a macro configuration and resolve all inter-macro dependencies.
    ///
    /// After a successful call here, macros are guaranteed to be fully recursively expanded. All
    /// instances of `Token::Identifier` that reference another macro in the set will be replaced
    /// with the fully expanded macro that the identifier references.
    ///
    /// Will return `Err` if any macro depends on itself recursively.
    ///
    /// This will expand using macros with the given named configuration if they exist, otherwise
    /// falling back to the default. If only the default configuration is desired, macro
    /// configuration cannot be the empty string, so providing the empty string here will always
    /// result in the default configuration.
    pub fn resolve<Q: ?Sized>(self, config: &Q) -> Result<ResolvedMacroSet<S>, RecursiveMacro>
    where
        S: Borrow<S> + Borrow<Q>,
        Q: Hash + Eq,
    {
        self.resolve_with_skip_recursive(config, |_| true)
    }

    /// A version of `MacroSet::resolve` that allows optionally skipping recursive expansion.
    ///
    /// For any token in a macro that matches another macro, if the `recursively_expand` callback
    /// returns false this token will *not* be recursively expanded. GMS2 skips such recursive
    /// expansion for builtin function names.
    pub fn resolve_with_skip_recursive<Q: ?Sized>(
        self,
        config: &Q,
        recursively_expand: impl Fn(&S) -> bool,
    ) -> Result<ResolvedMacroSet<S>, RecursiveMacro>
    where
        S: Borrow<S> + Borrow<Q>,
        Q: Hash + Eq,
    {
        // Determine a proper macro evaluation order.
        //
        // Add all of the macro indexes we need to evaluate to a stack.
        //
        // Take the top entry off of the stack and check to see if it has any un-evaluated
        // dependencies. If it does, then push the popped macro back onto the stack, followed
        // by all of its un-evaluated dependencies. Otherwise if the macro has no un-evaluated
        // dependencies, then it can be evaluated next.
        //
        // We also keep track of in-progress *evaluating* macros (un-evaluated macros that we have
        // encountered before which had an un-evaluated dependency). If we encounter one of these
        // macros more than once without it becoming evaluated in-between, then we know we have a
        // recursive macro.

        let mut eval_stack = (0..self.macros.len()).collect::<Vec<_>>();
        let mut evaluating_macros = IndexSet::new();
        let mut evaluated_macros = IndexSet::new();
        let mut eval_order = Vec::new();

        loop {
            let Some(macro_index) = eval_stack.pop() else {
                break;
            };

            if evaluated_macros.contains(macro_index) {
                // Macros can be in the evaluation stack more than once since we push every macro to
                // the stack first thing to make sure that every macro is evaluated. If we encounter
                // an evaluated macro again we can just skip it.
                continue;
            }

            let macro_ = &self.macros[macro_index];

            let mut has_unevaluated_dependency = false;
            for token in &macro_.tokens {
                if let TokenKind::Identifier(ident) = &token.kind {
                    if let Some(ind) = self
                        .macro_dict
                        .get(ident)
                        .and_then(|c| c.index_for_config(config))
                    {
                        if recursively_expand(ident) && !evaluated_macros.contains(ind) {
                            if !has_unevaluated_dependency {
                                has_unevaluated_dependency = true;

                                // We need to evaluate dependencies before this macro, so mark the
                                // macro as in-progress. If we get here a *second* time without
                                // the macro becoming evaluated, then we must have a recursive
                                // dependency.
                                if !evaluating_macros.insert(macro_index) {
                                    return Err(RecursiveMacro(macro_index));
                                }

                                // We must push the evaluating macro *before* all of its
                                // dependencies.
                                eval_stack.push(macro_index);
                            }

                            eval_stack.push(ind);
                        }
                    }
                }
            }

            if !has_unevaluated_dependency {
                evaluated_macros.insert(macro_index);
                eval_order.push(macro_index);
            }
        }

        // Once we have a known-good evaluation order, we can evaluate our interdependent macros in
        // this order and all references should be present.

        let mut resolved_macros = IndexMap::<Macro<S>>::new();

        for macro_index in eval_order {
            let mut expanded_tokens = Vec::new();
            let macro_ = &self.macros[macro_index];
            for token in &macro_.tokens {
                let ind = match &token.kind {
                    TokenKind::Identifier(ident) => {
                        if let Some(index) = self
                            .macro_dict
                            .get(ident)
                            .and_then(|c| c.index_for_config(config))
                        {
                            if recursively_expand(ident) {
                                Some(index)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                if let Some(ind) = ind {
                    expanded_tokens.extend_from_slice(
                        &resolved_macros
                            .get(ind)
                            .expect("bad macro evaluation order")
                            .tokens,
                    );
                } else {
                    expanded_tokens.push(token.clone());
                }
            }

            resolved_macros.insert(
                macro_index,
                Macro {
                    name: macro_.name.clone(),
                    config: macro_.config.clone(),
                    tokens: expanded_tokens,
                    span: macro_.span,
                },
            );
        }

        Ok(ResolvedMacroSet {
            macros: (0..self.macros.len())
                .map(|i| resolved_macros.remove(i).unwrap())
                .collect(),
            macro_dict: self
                .macro_dict
                .into_iter()
                .filter_map(|(k, v)| Some((k, v.index_for_config(config)?)))
                .collect(),
        })
    }

    /// Create an `MacroSet` with externally defined macros from the given [`SyntheticMacros`] set.
    pub fn with_synthetic(synthetic_macros: SyntheticMacros<S>) -> Self {
        let mut this = Self::default();

        for (i, (macro_name, tokens)) in synthetic_macros.into_iter().enumerate() {
            this.macro_dict.insert(
                macro_name.clone(),
                ConfigurationSet {
                    default: Some(i),
                    for_config: HashMap::new(),
                },
            );
            this.macros.push(Macro {
                name: macro_name,
                config: None,
                span: Span::null(),
                tokens,
            })
        }

        this
    }
}

/// A set of macros that have been evaluated for a specific configuration and with recursive macros
/// expanded.
#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct ResolvedMacroSet<S> {
    macros: Vec<Macro<S>>,
    macro_dict: HashMap<S, usize>,
}

impl<S> ResolvedMacroSet<S> {
    pub fn is_empty(&self) -> bool {
        self.macros.is_empty()
    }

    pub fn len(&self) -> usize {
        self.macros.len()
    }

    /// Get an extracted macro.
    pub fn get(&self, index: usize) -> Option<&Macro<S>> {
        self.macros.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Macro<S>> {
        self.macros.iter()
    }
}

impl<S: Eq + Hash> ResolvedMacroSet<S> {
    /// Find the index for a macro by name.
    pub fn find<Q: ?Sized>(&self, name: &Q) -> Option<usize>
    where
        S: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.macro_dict.get(name).copied()
    }
}

impl<S: Clone + Eq + Hash> ResolvedMacroSet<S> {
    /// Expand all macros in the given token list.
    pub fn expand(&self, tokens: &mut Vec<Token<S>>) {
        let mut expanded_tokens = Vec::new();

        for token in tokens.drain(..) {
            let macro_index = if let TokenKind::Identifier(i) = &token.kind {
                self.find(i)
            } else {
                None
            };

            if let Some(macro_index) = macro_index {
                // Set the span of every expanded token to be the span of the invoking token.
                expanded_tokens.extend(self.macros[macro_index].tokens.iter().map(|t| Token {
                    kind: t.kind.clone(),
                    span: token.span,
                }));
            } else {
                expanded_tokens.push(token);
            }
        }

        *tokens = expanded_tokens;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{lexer::Lexer, string_interner::StdStringInterner};

    #[test]
    fn test_macro_dependencies() {
        const SOURCE: &str = r#"
            #macro THREE TWO + ONE
            #macro correct:ONE 1
            #macro broken:ONE 2
            #macro FOUR TWO + TWO
            #macro TWO ONE + ONE
        "#;

        let mut tokens = Vec::new();
        Lexer::tokenize(StdStringInterner, SOURCE, &mut tokens).unwrap();

        let mut macros = MacroSet::new();

        macros.extract(&mut tokens).unwrap();
        let macros = macros.clone().resolve("correct").unwrap();

        assert_eq!(
            macros
                .get(macros.find("FOUR").unwrap())
                .unwrap()
                .tokens
                .iter()
                .map(|t| t.kind.clone())
                .collect::<Vec<_>>(),
            [
                TokenKind::Integer("1"),
                TokenKind::Plus,
                TokenKind::Integer("1"),
                TokenKind::Plus,
                TokenKind::Integer("1"),
                TokenKind::Plus,
                TokenKind::Integer("1"),
            ]
        )
    }

    #[test]
    fn test_recursive_macros() {
        const SOURCE: &str = r#"
            #macro ONE TWO
            #macro TWO ONE
        "#;

        let mut tokens = Vec::new();
        Lexer::tokenize(StdStringInterner, SOURCE, &mut tokens).unwrap();

        let mut macros = MacroSet::default();

        macros.extract(&mut tokens).unwrap();
        assert!(macros.resolve("").is_err());
    }
}
