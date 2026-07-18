use arrayvec::ArrayVec;
use fabricator_vm::Span;
use thiserror::Error;

use crate::{
    ast,
    constant::Constant,
    tokens::{Token, TokenKind},
};

#[derive(Debug, Error)]
pub enum ParseErrorKind {
    #[error("found {unexpected:?}, expected {expected:?}")]
    Unexpected {
        unexpected: &'static str,
        expected: &'static str,
    },
    #[error("end of token stream, expected {expected:?}")]
    EndOfStream { expected: &'static str },
    #[error("invalid numeric literal")]
    BadNumber,
    #[error("function declarations with inheritance must be annotated with `constructor`")]
    InheritWithoutConstructor,
    #[error("parser settings forbid `new` on call expressions")]
    NewDisallowed,
    #[error("accessor indexing is disallowed")]
    AccessorsDisallowed,
    #[error("`globalvar` declarations are disallowed")]
    GlobalVarDisallowed,
}

impl ParseErrorKind {
    /// Generate an `EndOfStream` parse error when the unexpected token is `EndOfStream`, otherwise
    /// generate an `Unexpected` parse error with the unexpected string as the token indicator
    /// string.
    fn unexpected_token<S>(t: &TokenKind<S>, expected: &'static str) -> ParseErrorKind {
        if matches!(t, TokenKind::EndOfStream) {
            ParseErrorKind::EndOfStream { expected }
        } else {
            ParseErrorKind::Unexpected {
                unexpected: token_indicator(t),
                expected,
            }
        }
    }
}

#[derive(Debug, Error)]
#[error("{kind}")]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
}

#[derive(Debug, Copy, Clone)]
pub struct ParseSettings {
    /// Require semicolons at the end of all statements other than block-like statements like `if`
    /// and `for`.
    pub strict_semicolons: bool,
    /// Allow `new` before function call expressions.
    pub allow_new: bool,
    /// Allow `|` `?` `#` `@` `$` accessors.
    pub allow_accessors: bool,
    /// Allow `globalvar` declarations.
    pub allow_globalvar: bool,
}

impl ParseSettings {
    pub fn strict() -> Self {
        ParseSettings {
            strict_semicolons: true,
            allow_new: false,
            allow_accessors: false,
            allow_globalvar: false,
        }
    }

    pub fn compat() -> Self {
        ParseSettings {
            strict_semicolons: false,
            allow_new: true,
            allow_accessors: true,
            allow_globalvar: true,
        }
    }

    pub fn parse<I, S>(self, token_iter: I) -> Result<ast::Block<S>, ParseError>
    where
        I: IntoIterator<Item = Token<S>>,
        S: AsRef<str>,
    {
        Parser::new(self, token_iter.into_iter()).parse()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum StatementTrailer {
    // Statement either must have or may have a trailing semicolon, depending on parser settings.
    SemiColon,
    // Statement must not have a trailing semicolon
    NoSemiColon,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum CommaSeparatedElement {
    // Element can be in any place in the comma separated list.
    Normal,
    // Element must be the final element in a comma separated list and cannot be followed by another
    // comma.
    Trailer,
}

struct BufferedToken<S> {
    token: Token<S>,
    follows_newline: bool,
}

struct Parser<I, S> {
    settings: ParseSettings,
    token_iter: I,
    look_ahead_buffer: ArrayVec<BufferedToken<S>, 1>,
    end_of_stream_span: Span,
}

impl<I, S> Parser<I, S>
where
    I: Iterator<Item = Token<S>>,
    S: AsRef<str>,
{
    fn new(settings: ParseSettings, token_iter: I) -> Self {
        Parser {
            settings,
            token_iter,
            look_ahead_buffer: ArrayVec::new(),
            end_of_stream_span: Span::null(),
        }
    }

    fn parse(&mut self) -> Result<ast::Block<S>, ParseError> {
        self.parse_block(|t| matches!(t, TokenKind::EndOfStream))
    }

    fn parse_block(
        &mut self,
        stop: impl Fn(&TokenKind<S>) -> bool,
    ) -> Result<ast::Block<S>, ParseError> {
        let mut statements = Vec::new();
        let mut span = Span::null();
        loop {
            self.look_ahead(1);
            let next = self.peek(0);
            if stop(&next.kind) {
                break;
            }

            if matches!(&next.kind, TokenKind::SemiColon) {
                self.advance(1);
                continue;
            }

            let stmt = self.parse_statement()?;
            span = span.combine(stmt.span());
            statements.push(stmt);
        }

        Ok(ast::Block { statements, span })
    }

    /// Parse a statement including any trailing semicolon, if it is expected.
    fn parse_statement(&mut self) -> Result<ast::Statement<S>, ParseError> {
        self.look_ahead(1);
        let &Token {
            kind: ref tok_kind,
            span: tok_span,
        } = self.peek(0);
        if matches!(tok_kind, TokenKind::SemiColon) {
            self.advance(1);
            Ok(ast::Statement::Empty(tok_span))
        } else {
            let (stmt, trailer) = self.parse_statement_body()?;

            match trailer {
                StatementTrailer::SemiColon => {
                    if self.settings.strict_semicolons {
                        self.parse_token(TokenKind::SemiColon)?;
                    } else {
                        self.look_ahead(1);
                        if matches!(self.peek(0).kind, TokenKind::SemiColon) {
                            self.advance(1);
                        }
                    }
                }
                StatementTrailer::NoSemiColon => {}
            }

            Ok(stmt)
        }
    }

    /// Parse a statement, not including any trailing semicolon.
    fn parse_statement_body(
        &mut self,
    ) -> Result<(ast::Statement<S>, StatementTrailer), ParseError> {
        self.look_ahead(1);
        let &Token {
            kind: ref tok_kind,
            span: tok_span,
        } = self.peek(0);

        Ok(match tok_kind {
            TokenKind::Enum => (
                ast::Statement::Enum(self.parse_enum_stmt()?),
                StatementTrailer::NoSemiColon,
            ),
            TokenKind::Function => (
                ast::Statement::Function(self.parse_function_stmt()?),
                StatementTrailer::NoSemiColon,
            ),
            TokenKind::Closure => (
                ast::Statement::Closure(self.parse_closure_stmt()?),
                StatementTrailer::NoSemiColon,
            ),
            TokenKind::Var => {
                self.advance(1);
                (
                    ast::Statement::Var(self.parse_var_declaration_list(tok_span)?),
                    StatementTrailer::SemiColon,
                )
            }
            TokenKind::Static => {
                self.advance(1);

                self.look_ahead(1);
                (
                    match *self.peek(0) {
                        Token {
                            kind: TokenKind::Let,
                            span: let_span,
                        } => {
                            self.advance(1);
                            ast::Statement::StaticLet(
                                self.parse_let_declaration_list(tok_span.combine(let_span))?,
                            )
                        }
                        _ => ast::Statement::Static(self.parse_var_declaration_list(tok_span)?),
                    },
                    StatementTrailer::SemiColon,
                )
            }
            TokenKind::Let => {
                self.advance(1);
                (
                    ast::Statement::Let(self.parse_let_declaration_list(tok_span)?),
                    StatementTrailer::SemiColon,
                )
            }
            TokenKind::GlobalVar => {
                self.advance(1);

                if !self.settings.allow_globalvar {
                    return Err(ParseError {
                        kind: ParseErrorKind::GlobalVarDisallowed,
                        span: tok_span,
                    });
                }

                let ident = self.parse_identifier()?;
                (
                    ast::Statement::GlobalVar(ident),
                    StatementTrailer::SemiColon,
                )
            }
            TokenKind::Return => {
                self.advance(1);
                let mut span = tok_span;

                self.look_ahead(1);
                let values = if matches!(self.peek(0).kind, TokenKind::SemiColon) {
                    Vec::new()
                } else {
                    let (values, values_span) = self.parse_expr_list()?;
                    span = span.combine(values_span);
                    values
                };

                (
                    ast::Statement::Return(ast::ReturnStmt { values, span }),
                    StatementTrailer::SemiColon,
                )
            }
            TokenKind::Exit => {
                self.advance(1);
                (ast::Statement::Exit(tok_span), StatementTrailer::SemiColon)
            }
            TokenKind::If => {
                self.advance(1);
                let condition = self.parse_expression()?;
                let then_stmt = self.parse_statement()?;
                let mut span = tok_span.combine(then_stmt.span());

                let mut else_stmt = None;

                self.look_ahead(1);
                let next = self.peek(0);
                if matches!(next.kind, TokenKind::Else) {
                    span = span.combine(next.span);
                    self.advance(1);
                    let stmt = self.parse_statement()?;
                    span = span.combine(stmt.span());
                    else_stmt = Some(stmt);
                }

                (
                    ast::Statement::If(ast::IfStmt {
                        condition: Box::new(condition),
                        then_stmt: Box::new(then_stmt),
                        else_stmt: else_stmt.map(Box::new),
                        span,
                    }),
                    StatementTrailer::NoSemiColon,
                )
            }
            TokenKind::For => (
                ast::Statement::For(self.parse_for_stmt()?),
                StatementTrailer::NoSemiColon,
            ),
            TokenKind::While => {
                self.advance(1);

                let condition = Box::new(self.parse_expression()?);
                let body = Box::new(self.parse_statement()?);

                let span = tok_span.combine(body.span());

                (
                    ast::Statement::While(ast::LoopStmt {
                        target: condition,
                        body,
                        span,
                    }),
                    StatementTrailer::NoSemiColon,
                )
            }
            TokenKind::Repeat => {
                self.advance(1);

                let times = Box::new(self.parse_expression()?);
                let body = Box::new(self.parse_statement()?);
                let span = tok_span.combine(body.span());

                (
                    ast::Statement::Repeat(ast::LoopStmt {
                        target: times,
                        body,
                        span,
                    }),
                    StatementTrailer::NoSemiColon,
                )
            }
            TokenKind::Switch => (
                ast::Statement::Switch(self.parse_switch_stmt()?),
                StatementTrailer::NoSemiColon,
            ),
            TokenKind::With => {
                self.advance(1);

                let target = Box::new(self.parse_expression()?);
                let body = Box::new(self.parse_statement()?);
                let span = tok_span.combine(body.span());

                (
                    ast::Statement::With(ast::LoopStmt { target, body, span }),
                    StatementTrailer::NoSemiColon,
                )
            }
            TokenKind::Try => {
                self.advance(1);
                let try_block = self.parse_statement()?;
                self.parse_token(TokenKind::Catch)?;
                self.parse_token(TokenKind::LeftParen)?;
                let err_ident = self.parse_identifier()?;
                self.parse_token(TokenKind::RightParen)?;
                let catch_block = self.parse_statement()?;
                let span = tok_span.combine(catch_block.span());
                (
                    ast::Statement::TryCatch(ast::TryCatchStmt {
                        try_block: Box::new(try_block),
                        err_ident,
                        catch_block: Box::new(catch_block),
                        span,
                    }),
                    StatementTrailer::NoSemiColon,
                )
            }
            TokenKind::Throw => {
                self.advance(1);
                let target = self.parse_expression()?;
                let span = tok_span.combine(target.span());
                (
                    ast::Statement::Throw(ast::ThrowStmt {
                        target: Box::new(target),
                        span,
                    }),
                    StatementTrailer::SemiColon,
                )
            }
            TokenKind::Break => {
                self.advance(1);
                (ast::Statement::Break(tok_span), StatementTrailer::SemiColon)
            }
            TokenKind::Continue => {
                self.advance(1);
                (
                    ast::Statement::Continue(tok_span),
                    StatementTrailer::SemiColon,
                )
            }
            TokenKind::LeftBrace => {
                self.advance(1);
                let block = self.parse_block(|t| matches!(t, TokenKind::RightBrace))?;
                let span = tok_span.combine(self.parse_token(TokenKind::RightBrace).unwrap());
                (
                    ast::Statement::Block(ast::BlockStmt { block, span }),
                    StatementTrailer::NoSemiColon,
                )
            }
            _ => {
                let expr = match self.parse_expression() {
                    Ok(expr) => expr,
                    Err(ParseError {
                        kind: ParseErrorKind::Unexpected { unexpected, .. },
                        span,
                    }) => {
                        return Err(ParseError {
                            kind: ParseErrorKind::Unexpected {
                                unexpected,
                                expected: "<statement>",
                            },
                            span,
                        });
                    }
                    Err(ParseError {
                        kind: ParseErrorKind::EndOfStream { .. },
                        span,
                    }) => {
                        return Err(ParseError {
                            kind: ParseErrorKind::EndOfStream {
                                expected: "<statement>",
                            },
                            span,
                        });
                    }
                    Err(err) => return Err(err),
                };

                let stmt = match expr {
                    ast::Expression::Prefix(mutation) => ast::Statement::Prefix(mutation),
                    ast::Expression::Postfix(mutation) => ast::Statement::Postfix(mutation),
                    ast::Expression::Call(call) => ast::Statement::Call(call),
                    expr => {
                        let mut span = expr.span();

                        let target = get_mutable_expr(expr).map_err(|_| ParseError {
                            kind: ParseErrorKind::Unexpected {
                                unexpected: "<non-statement expression>",
                                expected: "<statement>",
                            },
                            span,
                        })?;

                        self.look_ahead(1);
                        let Some(assignment_op) = get_assignment_operator(&self.peek(0).kind)
                        else {
                            return Err(ParseError {
                                kind: ParseErrorKind::Unexpected {
                                    unexpected: "<non-statement expression>",
                                    expected: "<statement>",
                                },
                                span,
                            });
                        };
                        self.advance(1);

                        let value = Box::new(self.parse_expression()?);
                        span = span.combine(value.span());

                        ast::Statement::Assignment(ast::AssignmentStmt {
                            target,
                            op: assignment_op,
                            value,
                            span,
                        })
                    }
                };

                (stmt, StatementTrailer::SemiColon)
            }
        })
    }

    fn parse_var_declaration_list(
        &mut self,
        decl_span: Span,
    ) -> Result<ast::VarDeclarationStmt<S>, ParseError> {
        let mut span = decl_span;
        let mut vars = Vec::new();
        loop {
            let name = self.parse_identifier()?;
            span = span.combine(name.span);

            self.look_ahead(1);
            let value;
            if matches!(self.peek(0).kind, TokenKind::Equal) {
                self.advance(1);
                let val = self.parse_expression()?;
                span = span.combine(val.span());
                value = Some(val);
            } else {
                value = None;
            }

            vars.push((name, value));

            self.look_ahead(1);
            if matches!(self.peek(0).kind, TokenKind::Comma) {
                self.advance(1)
            } else {
                break;
            }
        }

        Ok(ast::VarDeclarationStmt { vars, span })
    }

    fn parse_let_declaration_list(
        &mut self,
        decl_span: Span,
    ) -> Result<ast::LetDeclarationStmt<S>, ParseError> {
        let mut span = decl_span;
        let mut vars = Vec::new();

        loop {
            let name = self.parse_identifier()?;
            span = span.combine(name.span);

            vars.push(name);

            self.look_ahead(1);
            if matches!(self.peek(0).kind, TokenKind::Comma) {
                self.advance(1);
            } else {
                break;
            }
        }

        self.look_ahead(1);
        let Token {
            kind: ref next_kind,
            span: next_span,
        } = *self.peek(0);
        if matches!(next_kind, TokenKind::Equal) {
            self.advance(1);
            span = span.combine(next_span);

            let (exprs, exprs_span) = self.parse_expr_list()?;
            span = span.combine(exprs_span);

            Ok(ast::LetDeclarationStmt { vars, exprs, span })
        } else {
            Ok(ast::LetDeclarationStmt {
                vars,
                exprs: Vec::new(),
                span,
            })
        }
    }

    fn parse_expr_list(&mut self) -> Result<(Vec<ast::Expression<S>>, Span), ParseError> {
        let mut span = Span::null();
        let mut exprs = Vec::new();

        loop {
            let expr = self.parse_expression()?;
            span = span.combine(expr.span());

            exprs.push(expr);

            self.look_ahead(1);
            if matches!(self.peek(0).kind, TokenKind::Comma) {
                self.advance(1);
            } else {
                break;
            }
        }

        Ok((exprs, span))
    }

    fn parse_function_stmt(&mut self) -> Result<ast::FunctionStmt<S>, ParseError> {
        let mut span = self.parse_token(TokenKind::Function)?;
        let name = self.parse_identifier()?;
        let parameters = self.parse_parameter_list()?;

        self.look_ahead(1);
        let inherit = if matches!(self.peek(0).kind, TokenKind::Colon) {
            self.advance(1);
            let expr = self.parse_expression()?;

            let ast::Expression::Call(call_expr) = expr else {
                return Err(ParseError {
                    kind: ParseErrorKind::Unexpected {
                        unexpected: "<expression>",
                        expected: "<call expression>",
                    },
                    span: expr.span(),
                });
            };

            Some(call_expr)
        } else {
            None
        };

        self.look_ahead(1);
        let is_constructor = if matches!(self.peek(0).kind, TokenKind::Constructor) {
            self.advance(1);
            true
        } else {
            false
        };

        self.parse_token(TokenKind::LeftBrace)?;
        let body = self.parse_block(|t| matches!(t, TokenKind::RightBrace))?;
        span = span.combine(self.parse_token(TokenKind::RightBrace).unwrap());

        if !is_constructor && inherit.is_some() {
            return Err(ParseError {
                kind: ParseErrorKind::InheritWithoutConstructor,
                span,
            });
        }

        Ok(ast::FunctionStmt {
            name,
            is_constructor,
            inherit,
            parameters,
            body,
            span,
        })
    }

    fn parse_closure_stmt(&mut self) -> Result<ast::ClosureStmt<S>, ParseError> {
        let mut span = self.parse_token(TokenKind::Closure)?;
        let name = self.parse_identifier()?;
        let parameters = self.parse_parameter_list()?;

        self.parse_token(TokenKind::LeftBrace)?;
        let body = self.parse_block(|t| matches!(t, TokenKind::RightBrace))?;
        span = span.combine(self.parse_token(TokenKind::RightBrace).unwrap());

        Ok(ast::ClosureStmt {
            name,
            parameters,
            body,
            span,
        })
    }

    fn parse_enum_stmt(&mut self) -> Result<ast::EnumStmt<S>, ParseError> {
        let mut span = self.parse_token(TokenKind::Enum)?;
        let name = self.parse_identifier()?;

        let mut variants = Vec::new();

        span = span.combine(self.parse_comma_separated_list(
            TokenKind::LeftBrace,
            TokenKind::RightBrace,
            |this| {
                let key = this.parse_identifier()?;

                this.look_ahead(1);
                let value = if matches!(this.peek(0).kind, TokenKind::Equal) {
                    this.advance(1);
                    Some(this.parse_expression()?)
                } else {
                    None
                };

                variants.push((key, value));

                Ok(CommaSeparatedElement::Normal)
            },
        )?);

        Ok(ast::EnumStmt {
            name,
            variants,
            span,
        })
    }

    fn parse_for_stmt(&mut self) -> Result<ast::ForStmt<S>, ParseError> {
        let for_span = self.parse_token(TokenKind::For)?;
        self.parse_token(TokenKind::LeftParen)?;

        let check_tok = |this: &mut Self, tok: TokenKind<()>| -> Option<Span> {
            this.look_ahead(1);
            let &Token { ref kind, span } = this.peek(0);
            if kind.as_unit_string() == tok {
                Some(span.start_span())
            } else {
                None
            }
        };

        let initializer = if let Some(span) = check_tok(self, TokenKind::SemiColon) {
            ast::Statement::Empty(span)
        } else {
            self.parse_statement_body()?.0
        };

        self.parse_token(TokenKind::SemiColon)?;

        let condition = if let Some(span) = check_tok(self, TokenKind::SemiColon) {
            // As a special case, an empty condition is always true.
            ast::Expression::Constant(Constant::Boolean(true), span.start_span())
        } else {
            self.parse_expression()?
        };

        self.parse_token(TokenKind::SemiColon)?;

        let iterator = if let Some(span) = check_tok(self, TokenKind::RightParen) {
            ast::Statement::Empty(span)
        } else {
            self.parse_statement_body()?.0
        };

        if check_tok(self, TokenKind::SemiColon).is_some() {
            self.advance(1);
        }

        self.parse_token(TokenKind::RightParen)?;

        let body = self.parse_statement()?;

        let span = for_span.combine(body.span());

        Ok(ast::ForStmt {
            initializer: Box::new(initializer),
            condition: Box::new(condition),
            iterator: Box::new(iterator),
            body: Box::new(body),
            span,
        })
    }

    fn parse_switch_stmt(&mut self) -> Result<ast::SwitchStmt<S>, ParseError> {
        let mut span = self.parse_token(TokenKind::Switch)?;
        let target = self.parse_expression()?;
        self.parse_token(TokenKind::LeftBrace)?;

        let mut cases = Vec::new();
        let mut default = None;

        loop {
            self.look_ahead(1);
            let next = self.peek(0);

            if matches!(&next.kind, TokenKind::RightBrace) {
                span = span.combine(next.span);
                self.advance(1);
                break;
            } else if default.is_some() {
                return Err(ParseError {
                    kind: ParseErrorKind::unexpected_token(
                        &next.kind,
                        token_indicator::<()>(&TokenKind::RightBrace),
                    ),
                    span: next.span,
                });
            }

            match &next.kind {
                TokenKind::Case => {
                    let mut span = next.span;

                    self.advance(1);
                    let compare = self.parse_expression()?;
                    self.parse_token(TokenKind::Colon)?;
                    let body = self.parse_block(|t| {
                        matches!(
                            t,
                            TokenKind::Case | TokenKind::Default | TokenKind::RightBrace
                        )
                    })?;
                    span = span.combine(body.span);
                    cases.push(ast::SwitchCase {
                        compare,
                        body,
                        span,
                    });
                }
                TokenKind::Default => {
                    self.advance(1);
                    self.parse_token(TokenKind::Colon)?;
                    default = Some(self.parse_block(|t| {
                        matches!(
                            t,
                            TokenKind::Case | TokenKind::Default | TokenKind::RightBrace
                        )
                    })?);
                }
                token => {
                    return Err(ParseError {
                        kind: ParseErrorKind::unexpected_token(token, "<switch statement case>"),
                        span,
                    });
                }
            }
        }

        Ok(ast::SwitchStmt {
            target: Box::new(target),
            cases,
            default,
            span,
        })
    }

    fn parse_expression(&mut self) -> Result<ast::Expression<S>, ParseError> {
        let mut expr = self.parse_sub_expression(MIN_PRIORITY)?;

        // Handle ternary operators, which have lower precedence than all unary and binary operators.
        loop {
            self.look_ahead(1);
            if !matches!(self.peek(0).kind, TokenKind::QuestionMark) {
                break;
            }

            self.advance(1);

            let cond = expr;
            let if_true = self.parse_expression()?;
            self.parse_token(TokenKind::Colon)?;
            let if_false = self.parse_expression()?;

            let span = cond.span().combine(if_false.span());
            expr = ast::Expression::Ternary(ast::TernaryExpr {
                cond: Box::new(cond),
                if_true: Box::new(if_true),
                if_false: Box::new(if_false),
                span,
            });
        }

        Ok(expr)
    }

    fn parse_sub_expression(
        &mut self,
        priority_limit: OperatorPriority,
    ) -> Result<ast::Expression<S>, ParseError> {
        self.look_ahead(1);
        let &Token {
            kind: ref tok_kind,
            span: tok_span,
        } = self.peek(0);

        let mut expr = if let Some(prefix_op) = get_mutation_operator(tok_kind) {
            self.advance(1);
            let target = self.parse_sub_expression(UNARY_PRIORITY)?;
            let span = tok_span.combine(target.span());
            let target = get_mutable_expr(target)?;
            ast::Expression::Prefix(ast::Mutation {
                op: prefix_op,
                target: Box::new(target),
                span,
            })
        } else if let Some(unary_op) = get_unary_operator(tok_kind) {
            self.advance(1);
            let target = self.parse_sub_expression(UNARY_PRIORITY)?;
            let span = tok_span.combine(target.span());
            ast::Expression::Unary(ast::UnaryExpr {
                op: unary_op,
                target: Box::new(target),
                span,
            })
        } else {
            self.parse_simple_expression()?
        };

        loop {
            self.look_ahead(1);
            let Some(binary_op) = get_binary_operator(&self.peek(0).kind) else {
                break;
            };

            let (left_priority, right_priority) = binary_priority(binary_op);
            if left_priority <= priority_limit {
                break;
            }

            self.advance(1);

            let right_expression = self.parse_sub_expression(right_priority)?;
            let span = expr.span().combine(right_expression.span());
            expr = ast::Expression::Binary(ast::BinaryExpr {
                left: Box::new(expr),
                op: binary_op,
                right: Box::new(right_expression),
                span,
            });
        }

        Ok(expr)
    }

    fn parse_suffixed_expression(&mut self) -> Result<ast::Expression<S>, ParseError> {
        let mut expr = self.parse_primary_expression()?;
        loop {
            self.look_ahead(1);
            let &Token {
                kind: ref tok_kind,
                span: tok_span,
            } = self.peek(0);
            let tok_follows_newline = self.peek_newline(0);
            match tok_kind {
                TokenKind::LeftParen => {
                    let mut arguments = Vec::new();

                    let span = expr.span().combine(self.parse_comma_separated_list(
                        TokenKind::LeftParen,
                        TokenKind::RightParen,
                        |this| {
                            arguments.push(this.parse_expression()?);
                            Ok(CommaSeparatedElement::Normal)
                        },
                    )?);

                    expr = ast::Expression::Call(ast::Call {
                        base: Box::new(expr),
                        arguments,
                        has_new: false,
                        span,
                    });
                }
                TokenKind::Dot => {
                    self.advance(1);
                    let field = self.parse_identifier()?;
                    let span = expr.span().combine(field.span);
                    expr = ast::Expression::Field(ast::FieldExpr {
                        base: Box::new(expr),
                        field,
                        span,
                    });
                }
                TokenKind::LeftBracket => {
                    self.advance(1);

                    self.look_ahead(1);
                    let &Token {
                        kind: ref tok_kind,
                        span: tok_span,
                    } = self.peek(0);
                    let accessor_type = if let Some(accessor_type) = get_accessor_type(tok_kind) {
                        self.advance(1);
                        Some(accessor_type)
                    } else {
                        None
                    };

                    if accessor_type.is_some() && !self.settings.allow_accessors {
                        return Err(ParseError {
                            kind: ParseErrorKind::AccessorsDisallowed,
                            span: tok_span,
                        });
                    }

                    let mut indexes = Vec::new();

                    loop {
                        let index = self.parse_expression()?;
                        indexes.push(index);

                        self.look_ahead(1);
                        if matches!(self.peek(0).kind, TokenKind::Comma) {
                            self.advance(1);
                        } else {
                            break;
                        }
                    }

                    let span = expr
                        .span()
                        .combine(self.parse_token(TokenKind::RightBracket)?);
                    expr = ast::Expression::Index(ast::IndexExpr {
                        base: Box::new(expr),
                        accessor_type,
                        indexes,
                        span,
                    });
                }
                token => {
                    // Postfix operators cannot be separated by a newline.
                    if let Some(postfix_op) = get_mutation_operator(token)
                        && !tok_follows_newline
                    {
                        let span = expr.span().combine(tok_span);
                        let target = get_mutable_expr(expr)?;
                        self.advance(1);
                        expr = ast::Expression::Postfix(ast::Mutation {
                            target: Box::new(target),
                            op: postfix_op,
                            span,
                        });
                    } else {
                        break;
                    }
                }
            }
        }
        Ok(expr)
    }

    fn parse_primary_expression(&mut self) -> Result<ast::Expression<S>, ParseError> {
        let Token {
            kind: tok_kind,
            span: tok_span,
        } = self.next();
        match tok_kind {
            TokenKind::LeftParen => {
                let expr = self.parse_expression()?;
                let span = tok_span.combine(self.parse_token(TokenKind::RightParen)?);
                Ok(ast::Expression::Group(ast::GroupExpr {
                    inner: Box::new(expr),
                    span,
                }))
            }
            TokenKind::Identifier(n) => Ok(ast::Expression::Ident(ast::Ident::new(n, tok_span))),
            TokenKind::Global => Ok(ast::Expression::Global(tok_span)),
            TokenKind::This => Ok(ast::Expression::This(tok_span)),
            TokenKind::Other => Ok(ast::Expression::Other(tok_span)),
            TokenKind::Argument => {
                // `argument` can ONLY be referenced like `argument[{index}]`.
                self.parse_token(TokenKind::LeftBracket)?;
                let arg_index = self.parse_expression()?;
                let span = tok_span.combine(self.parse_token(TokenKind::RightBracket)?);
                Ok(ast::Expression::Argument(ast::ArgumentExpr {
                    arg_index: Box::new(arg_index),
                    span,
                }))
            }
            TokenKind::ArgumentCount => Ok(ast::Expression::ArgumentCount(tok_span)),
            token => Err(ParseError {
                kind: ParseErrorKind::unexpected_token(&token, "<grouped expression or name>"),
                span: tok_span,
            }),
        }
    }

    fn parse_simple_expression(&mut self) -> Result<ast::Expression<S>, ParseError> {
        self.look_ahead(1);
        let &Token {
            kind: ref tok_kind,
            span: tok_span,
        } = self.peek(0);
        match tok_kind {
            TokenKind::Undefined => {
                self.advance(1);
                Ok(ast::Expression::Constant(Constant::Undefined, tok_span))
            }
            TokenKind::True => {
                self.advance(1);
                Ok(ast::Expression::Constant(Constant::Boolean(true), tok_span))
            }
            TokenKind::False => {
                self.advance(1);
                Ok(ast::Expression::Constant(
                    Constant::Boolean(false),
                    tok_span,
                ))
            }
            TokenKind::Integer(i) => {
                let s = i.as_ref().replace('_', "");
                self.next();
                match i64::from_str_radix(&s, 10) {
                    Ok(i) => Ok(ast::Expression::Constant(Constant::Integer(i), tok_span)),
                    Err(_) => Err(ParseError {
                        kind: ParseErrorKind::BadNumber,
                        span: tok_span,
                    }),
                }
            }
            TokenKind::HexInteger(i) => {
                let s = i.as_ref();
                assert!(s[0..2].eq_ignore_ascii_case("0x"));
                let s = &s[2..].replace('_', "");
                self.next();
                match i64::from_str_radix(&s, 16) {
                    Ok(i) => Ok(ast::Expression::Constant(Constant::Integer(i), tok_span)),
                    Err(_) => Err(ParseError {
                        kind: ParseErrorKind::BadNumber,
                        span: tok_span,
                    }),
                }
            }
            TokenKind::DollarHexInteger(i) => {
                let s = i.as_ref();
                assert!(s[0..1].eq_ignore_ascii_case("$"));
                let s = &s[1..].replace('_', "");
                self.next();
                match i64::from_str_radix(&s, 16) {
                    Ok(i) => Ok(ast::Expression::Constant(Constant::Integer(i), tok_span)),
                    Err(_) => Err(ParseError {
                        kind: ParseErrorKind::BadNumber,
                        span: tok_span,
                    }),
                }
            }
            TokenKind::Float(f) => {
                let s = f.as_ref().replace('_', "");
                self.next();
                match s.parse::<f64>() {
                    Ok(f) => Ok(ast::Expression::Constant(Constant::Float(f), tok_span)),
                    Err(_) => Err(ParseError {
                        kind: ParseErrorKind::BadNumber,
                        span: tok_span,
                    }),
                }
            }
            TokenKind::String(_) => {
                let Token {
                    kind: TokenKind::String(s),
                    ..
                } = self.next()
                else {
                    unreachable!()
                };
                Ok(ast::Expression::Constant(Constant::String(s), tok_span))
            }
            TokenKind::Function => {
                self.advance(1);

                let parameters = self.parse_parameter_list()?;

                self.look_ahead(1);
                let inherit = if matches!(self.peek(0).kind, TokenKind::Colon) {
                    self.advance(1);
                    let expr = self.parse_expression()?;

                    let ast::Expression::Call(call_expr) = expr else {
                        return Err(ParseError {
                            kind: ParseErrorKind::Unexpected {
                                unexpected: "<expression>",
                                expected: "<call expression>",
                            },
                            span: expr.span(),
                        });
                    };

                    Some(call_expr)
                } else {
                    None
                };

                self.look_ahead(1);
                let is_constructor = if matches!(self.peek(0).kind, TokenKind::Constructor) {
                    self.advance(1);
                    true
                } else {
                    false
                };

                self.parse_token(TokenKind::LeftBrace)?;
                let body = self.parse_block(|t| matches!(t, TokenKind::RightBrace))?;
                let span = tok_span.combine(self.parse_token(TokenKind::RightBrace).unwrap());

                if !is_constructor && inherit.is_some() {
                    return Err(ParseError {
                        kind: ParseErrorKind::InheritWithoutConstructor,
                        span,
                    });
                }

                Ok(ast::Expression::Function(ast::FunctionExpr {
                    is_constructor,
                    inherit,
                    parameters,
                    body,
                    span,
                }))
            }
            TokenKind::Closure => {
                self.advance(1);

                let parameters = self.parse_parameter_list()?;

                self.parse_token(TokenKind::LeftBrace)?;
                let body = self.parse_block(|t| matches!(t, TokenKind::RightBrace))?;
                let span = tok_span.combine(self.parse_token(TokenKind::RightBrace).unwrap());

                Ok(ast::Expression::Closure(ast::ClosureExpr {
                    parameters,
                    body,
                    span,
                }))
            }
            TokenKind::New => {
                self.advance(1);

                let mut expr = self.parse_expression()?;
                let expr_span = expr.span();

                match &mut expr {
                    ast::Expression::Call(call_expr) => {
                        call_expr.span = tok_span.combine(expr_span);
                        if !self.settings.allow_new {
                            return Err(ParseError {
                                kind: ParseErrorKind::NewDisallowed,
                                span: expr.span(),
                            });
                        }

                        call_expr.has_new = true;
                        Ok(expr)
                    }
                    _ => Err(ParseError {
                        kind: ParseErrorKind::Unexpected {
                            unexpected: "<suffixed expression>",
                            expected: "<call expression>",
                        },
                        span: expr.span(),
                    }),
                }
            }
            TokenKind::LeftBrace => Ok(ast::Expression::Object(self.parse_object()?)),
            TokenKind::LeftBracket => Ok(ast::Expression::Array(self.parse_array()?)),
            TokenKind::DotDotDot => {
                self.advance(1);
                Ok(ast::Expression::VarArgs(tok_span))
            }
            _ => self.parse_suffixed_expression(),
        }
    }

    fn parse_object(&mut self) -> Result<ast::ObjectExpr<S>, ParseError> {
        let mut fields = Vec::new();

        let span =
            self.parse_comma_separated_list(TokenKind::LeftBrace, TokenKind::RightBrace, |this| {
                let key = this.parse_identifier()?;

                this.look_ahead(1);
                if matches!(this.peek(0).kind, TokenKind::Colon) {
                    this.parse_token(TokenKind::Colon)?;
                    let value = this.parse_expression()?;
                    fields.push(ast::Field::Value(key, value));
                } else {
                    fields.push(ast::Field::Init(key));
                }

                Ok(CommaSeparatedElement::Normal)
            })?;

        Ok(ast::ObjectExpr { fields, span })
    }

    fn parse_array(&mut self) -> Result<ast::ArrayExpr<S>, ParseError> {
        let mut entries = Vec::new();

        let span = self.parse_comma_separated_list(
            TokenKind::LeftBracket,
            TokenKind::RightBracket,
            |this| {
                entries.push(this.parse_expression()?);
                Ok(CommaSeparatedElement::Normal)
            },
        )?;

        Ok(ast::ArrayExpr { entries, span })
    }

    fn parse_parameter_list(&mut self) -> Result<ast::ParameterList<S>, ParseError> {
        let mut parameters = Vec::new();
        let mut var_args = None;
        let span =
            self.parse_comma_separated_list(TokenKind::LeftParen, TokenKind::RightParen, |this| {
                this.look_ahead(1);
                let next_token = this.peek(0);
                if matches!(next_token.kind, TokenKind::DotDotDot) {
                    var_args = Some(next_token.span);
                    this.advance(1);
                    Ok(CommaSeparatedElement::Trailer)
                } else {
                    let name = this.parse_identifier()?;
                    let mut default = None;
                    let mut span = name.span;

                    this.look_ahead(1);
                    if matches!(this.peek(0).kind, TokenKind::Equal) {
                        this.advance(1);
                        let expr = this.parse_expression()?;
                        span = span.combine(expr.span());
                        default = Some(expr);
                    }

                    parameters.push(ast::Parameter {
                        name,
                        default,
                        span,
                    });

                    Ok(CommaSeparatedElement::Normal)
                }
            })?;

        Ok(ast::ParameterList {
            fixed: parameters,
            var_args,
            span,
        })
    }

    /// Parse a comma separated list of items surrounded by paired left / right delimiters.
    ///
    /// Takes a callback to parse whatever the *item* is.
    fn parse_comma_separated_list(
        &mut self,
        left_delimiter: TokenKind<()>,
        right_delimiter: TokenKind<()>,
        mut read_item: impl FnMut(&mut Self) -> Result<CommaSeparatedElement, ParseError>,
    ) -> Result<Span, ParseError> {
        let mut span = self.parse_token(left_delimiter)?;

        let is_right_delimiter = |kind: &TokenKind<S>| kind.as_unit_string() == right_delimiter;

        loop {
            self.look_ahead(1);
            if is_right_delimiter(&self.peek(0).kind) {
                break;
            }

            let item_type = read_item(self)?;

            self.look_ahead(1);
            let next = self.peek(0);

            match (item_type, &next.kind) {
                (CommaSeparatedElement::Normal, TokenKind::Comma) => {
                    self.advance(1);
                }
                (_, kind) if is_right_delimiter(kind) => {
                    break;
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::unexpected_token(
                            &next.kind,
                            "',' or <right delimiter>",
                        ),
                        span: next.span,
                    });
                }
            }
        }

        span = span.combine(self.parse_token(right_delimiter)?);
        Ok(span)
    }

    fn parse_identifier(&mut self) -> Result<ast::Ident<S>, ParseError> {
        let Token { kind, span } = self.next();
        match kind {
            TokenKind::Identifier(ident) => Ok(ast::Ident { inner: ident, span }),
            t => Err(ParseError {
                kind: ParseErrorKind::unexpected_token(&t, "<identifier>"),
                span,
            }),
        }
    }

    fn parse_token(&mut self, expected: TokenKind<()>) -> Result<Span, ParseError> {
        let Token { kind, span } = self.next();
        if kind.as_unit_string() == expected {
            Ok(span)
        } else {
            Err(ParseError {
                kind: ParseErrorKind::unexpected_token(&kind, token_indicator(&expected)),
                span,
            })
        }
    }

    // Look ahead `n` tokens in the lexer, making them available to peek methods.
    fn look_ahead(&mut self, n: usize) {
        let mut follows_newline = false;
        while self.look_ahead_buffer.len() < n {
            match self.token_iter.next() {
                Some(token) => {
                    if matches!(token.kind, TokenKind::EndOfStream) {
                        // If our token stream has a real `EndOfStream` token, record its span so
                        // that all generated `EndOfStream` tokens will have the correct span.
                        self.end_of_stream_span = token.span;
                    }

                    if matches!(token.kind, TokenKind::Newline) {
                        follows_newline = true;
                    } else {
                        self.look_ahead_buffer.push(BufferedToken {
                            token,
                            follows_newline,
                        });
                        follows_newline = false;
                    }
                }
                None => {
                    // If the token stream does not generate an `EndOfStream` token, the span here
                    // will be null.
                    self.look_ahead_buffer.push(BufferedToken {
                        token: Token {
                            kind: TokenKind::EndOfStream,
                            span: self.end_of_stream_span,
                        },
                        follows_newline,
                    });
                    follows_newline = false;
                }
            }
        }
    }

    // Advance the token stream `n` tokens.
    //
    // # Panics
    //
    // Panics if the given `n` is less than the number of tokens we have previously buffered with
    // `Parser::look_ahead`.
    fn advance(&mut self, n: usize) {
        self.look_ahead_buffer.drain(0..n);
    }

    // Returns a reference to the `n`th token ahead in the look-ahead token buffer if it exists,
    // along with the line number on which it is found.
    //
    // # Panics
    //
    // Panics if the given `n` is less than the number of tokens we have previously buffered with
    // `Parser::look_ahead`.
    fn peek(&self, n: usize) -> &Token<S> {
        &self.look_ahead_buffer[n].token
    }

    /// Return true if the `n`th token ahead in the look-ahead buffer follows a newline.
    fn peek_newline(&self, n: usize) -> bool {
        self.look_ahead_buffer[n].follows_newline
    }

    // Return the next token in the token stream if it exists and advance the stream.
    fn next(&mut self) -> Token<S> {
        self.look_ahead(1);
        self.look_ahead_buffer.remove(0).token
    }
}

fn get_mutable_expr<S>(expr: ast::Expression<S>) -> Result<ast::MutableExpr<S>, ParseError> {
    match expr {
        ast::Expression::Ident(name) => Ok(ast::MutableExpr::Ident(name)),
        ast::Expression::Field(field_expr) => Ok(ast::MutableExpr::Field(field_expr)),
        ast::Expression::Index(index_expr) => Ok(ast::MutableExpr::Index(index_expr)),
        ast::Expression::Group(expr) => get_mutable_expr(*expr.inner),
        expr => Err(ParseError {
            kind: ParseErrorKind::Unexpected {
                unexpected: "<immutable expression>",
                expected: "<mutable expression>",
            },
            span: expr.span(),
        }),
    }
}

fn get_unary_operator<S>(token: &TokenKind<S>) -> Option<ast::UnaryOp> {
    match *token {
        TokenKind::Minus => Some(ast::UnaryOp::Minus),
        TokenKind::Bang => Some(ast::UnaryOp::Not),
        TokenKind::Tilde => Some(ast::UnaryOp::BitNegate),
        _ => None,
    }
}

fn get_mutation_operator<S>(token: &TokenKind<S>) -> Option<ast::MutationOp> {
    match *token {
        TokenKind::DoublePlus => Some(ast::MutationOp::Increment),
        TokenKind::DoubleMinus => Some(ast::MutationOp::Decrement),
        _ => None,
    }
}

fn get_binary_operator<S>(token: &TokenKind<S>) -> Option<ast::BinaryOp> {
    match *token {
        TokenKind::Plus => Some(ast::BinaryOp::Add),
        TokenKind::Minus => Some(ast::BinaryOp::Sub),
        TokenKind::Star => Some(ast::BinaryOp::Mult),
        TokenKind::Slash => Some(ast::BinaryOp::Div),
        TokenKind::Percent => Some(ast::BinaryOp::Mod),
        TokenKind::Mod => Some(ast::BinaryOp::Rem),
        TokenKind::Div => Some(ast::BinaryOp::IDiv),
        TokenKind::And => Some(ast::BinaryOp::And),
        TokenKind::Or => Some(ast::BinaryOp::Or),
        TokenKind::Xor => Some(ast::BinaryOp::Xor),
        TokenKind::DoubleEqual => Some(ast::BinaryOp::Equal),
        TokenKind::BangEqual => Some(ast::BinaryOp::NotEqual),
        TokenKind::Less => Some(ast::BinaryOp::LessThan),
        TokenKind::LessEqual => Some(ast::BinaryOp::LessEqual),
        TokenKind::Greater => Some(ast::BinaryOp::GreaterThan),
        TokenKind::GreaterEqual => Some(ast::BinaryOp::GreaterEqual),
        TokenKind::DoubleAmpersand => Some(ast::BinaryOp::And),
        TokenKind::DoublePipe => Some(ast::BinaryOp::Or),
        TokenKind::DoubleCaret => Some(ast::BinaryOp::Xor),
        TokenKind::Ampersand => Some(ast::BinaryOp::BitAnd),
        TokenKind::Pipe => Some(ast::BinaryOp::BitOr),
        TokenKind::Caret => Some(ast::BinaryOp::BitXor),
        TokenKind::DoubleLess => Some(ast::BinaryOp::BitShiftLeft),
        TokenKind::DoubleGreater => Some(ast::BinaryOp::BitShiftRight),
        TokenKind::DoubleQuestionMark => Some(ast::BinaryOp::NullCoalesce),
        _ => None,
    }
}

fn get_assignment_operator<S>(token: &TokenKind<S>) -> Option<ast::AssignmentOp> {
    match *token {
        TokenKind::Equal => Some(ast::AssignmentOp::Equal),
        TokenKind::PlusEqual => Some(ast::AssignmentOp::PlusEqual),
        TokenKind::MinusEqual => Some(ast::AssignmentOp::MinusEqual),
        TokenKind::StarEqual => Some(ast::AssignmentOp::MultEqual),
        TokenKind::SlashEqual => Some(ast::AssignmentOp::DivEqual),
        TokenKind::PercentEqual => Some(ast::AssignmentOp::RemEqual),
        TokenKind::AmpersandEqual => Some(ast::AssignmentOp::BitAndEqual),
        TokenKind::PipeEqual => Some(ast::AssignmentOp::BitOrEqual),
        TokenKind::CaretEqual => Some(ast::AssignmentOp::BitXorEqual),
        TokenKind::DoubleQuestionMarkEqual => Some(ast::AssignmentOp::NullCoalesce),
        _ => None,
    }
}

fn get_accessor_type<S>(token: &TokenKind<S>) -> Option<ast::AccessorType> {
    match *token {
        TokenKind::Pipe => Some(ast::AccessorType::List),
        TokenKind::QuestionMark => Some(ast::AccessorType::Map),
        TokenKind::Octothorpe => Some(ast::AccessorType::Grid),
        TokenKind::AtSign => Some(ast::AccessorType::Array),
        TokenKind::Dollar => Some(ast::AccessorType::Struct),
        _ => None,
    }
}

fn token_indicator<S>(t: &TokenKind<S>) -> &'static str {
    match *t {
        TokenKind::EndOfStream => "<eof>",
        TokenKind::Newline => "<nl>",
        TokenKind::Macro => "#macro",
        TokenKind::LeftParen => "(",
        TokenKind::RightParen => ")",
        TokenKind::LeftBracket => "[",
        TokenKind::RightBracket => "]",
        TokenKind::LeftBrace => "{",
        TokenKind::RightBrace => "}",
        TokenKind::Colon => ":",
        TokenKind::SemiColon => ";",
        TokenKind::Comma => ",",
        TokenKind::Dot => ".",
        TokenKind::DotDotDot => "...",
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Bang => "!",
        TokenKind::Slash => "/",
        TokenKind::Star => "*",
        TokenKind::Percent => "%",
        TokenKind::Ampersand => "&",
        TokenKind::Pipe => "|",
        TokenKind::Tilde => "~",
        TokenKind::Caret => "^",
        TokenKind::Div => "div",
        TokenKind::Mod => "mod",
        TokenKind::And => "and",
        TokenKind::Or => "or",
        TokenKind::Xor => "xor",
        TokenKind::QuestionMark => "?",
        TokenKind::Octothorpe => "#",
        TokenKind::AtSign => "@",
        TokenKind::Dollar => "$",
        TokenKind::Equal => "=",
        TokenKind::PlusEqual => "+=",
        TokenKind::MinusEqual => "-=",
        TokenKind::StarEqual => "*=",
        TokenKind::SlashEqual => "/=",
        TokenKind::PercentEqual => "%=",
        TokenKind::AmpersandEqual => "&=",
        TokenKind::PipeEqual => "|=",
        TokenKind::CaretEqual => "^=",
        TokenKind::DoubleQuestionMarkEqual => "??=",
        TokenKind::DoubleEqual => "==",
        TokenKind::BangEqual => "!=",
        TokenKind::Less => "<",
        TokenKind::LessEqual => "<=",
        TokenKind::Greater => ">",
        TokenKind::GreaterEqual => ">=",
        TokenKind::DoubleQuestionMark => "??",
        TokenKind::DoublePlus => "++",
        TokenKind::DoubleMinus => "--",
        TokenKind::DoubleAmpersand => "&&",
        TokenKind::DoublePipe => "||",
        TokenKind::DoubleCaret => "^^",
        TokenKind::DoubleLess => "<<",
        TokenKind::DoubleGreater => ">>",
        TokenKind::Enum => "enum",
        TokenKind::Function => "function",
        TokenKind::Closure => "closure",
        TokenKind::Constructor => "constructor",
        TokenKind::Var => "var",
        TokenKind::Let => "let",
        TokenKind::Static => "static",
        TokenKind::GlobalVar => "globalvar",
        TokenKind::Switch => "switch",
        TokenKind::Case => "case",
        TokenKind::Default => "default",
        TokenKind::Break => "break",
        TokenKind::Continue => "continue",
        TokenKind::If => "if",
        TokenKind::Else => "else",
        TokenKind::For => "for",
        TokenKind::Repeat => "repeat",
        TokenKind::While => "While",
        TokenKind::With => "With",
        TokenKind::Return => "return",
        TokenKind::Exit => "exit",
        TokenKind::Throw => "throw",
        TokenKind::Try => "try",
        TokenKind::Catch => "catch",
        TokenKind::Finally => "finally",
        TokenKind::Undefined => "undefined",
        TokenKind::True => "true",
        TokenKind::False => "false",
        TokenKind::Global => "global",
        TokenKind::This => "self",
        TokenKind::Other => "other",
        TokenKind::New => "new",
        TokenKind::Argument => "argument",
        TokenKind::ArgumentCount => "argument_count",
        TokenKind::Integer(_) => "<integer>",
        TokenKind::HexInteger(_) => "<hex_integer>",
        TokenKind::DollarHexInteger(_) => "<dollar_hex_integer>",
        TokenKind::Float(_) => "<float>",
        TokenKind::Identifier(_) => "<identifier>",
        TokenKind::String(_) => "<string>",
    }
}

type OperatorPriority = u8;

// Priority lower than any unary or binary operator.
const MIN_PRIORITY: OperatorPriority = 0;

// Priority of all unary operators.
const UNARY_PRIORITY: OperatorPriority = 12;

// Returns the left and right priority of the given binary operator.
//
// Different left and right priorities can be used to make an operation associate leftwards
// or rightwards, if the two priorities are the same the operation will default to associating
// leftwards.
fn binary_priority(operator: ast::BinaryOp) -> (OperatorPriority, OperatorPriority) {
    match operator {
        ast::BinaryOp::Mult => (11, 11),
        ast::BinaryOp::Div => (11, 11),
        ast::BinaryOp::Mod => (11, 11),
        ast::BinaryOp::Rem => (11, 11),
        ast::BinaryOp::IDiv => (11, 11),
        ast::BinaryOp::Add => (10, 10),
        ast::BinaryOp::Sub => (10, 10),
        ast::BinaryOp::BitShiftLeft => (9, 9),
        ast::BinaryOp::BitShiftRight => (9, 9),
        ast::BinaryOp::LessThan => (8, 8),
        ast::BinaryOp::LessEqual => (8, 8),
        ast::BinaryOp::GreaterThan => (8, 8),
        ast::BinaryOp::GreaterEqual => (8, 8),
        ast::BinaryOp::Equal => (7, 7),
        ast::BinaryOp::NotEqual => (7, 7),
        ast::BinaryOp::BitAnd => (6, 6),
        ast::BinaryOp::BitXor => (5, 5),
        ast::BinaryOp::BitOr => (4, 4),
        ast::BinaryOp::And => (3, 3),
        ast::BinaryOp::Xor => (2, 2),
        ast::BinaryOp::Or => (1, 1),
        ast::BinaryOp::NullCoalesce => (1, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{lexer::Lexer, string_interner::StringInterner};

    fn parse(settings: ParseSettings, source: &str) -> Result<ast::Block<String>, ParseError> {
        struct SimpleInterner;

        impl StringInterner for SimpleInterner {
            type String = String;

            fn intern(&mut self, s: &str) -> Self::String {
                s.to_owned()
            }
        }

        let mut tokens = Vec::new();
        Lexer::tokenize(SimpleInterner, source, &mut tokens).unwrap();

        settings.parse(tokens)
    }

    #[test]
    fn test_parser() {
        const SOURCE: &str = r#"
            // Line comment
            var sum = 0;
            for (var i = 0; i < 1000000; i += 1) {
                /*
                    Multiline comment
                */
                sum += i;
            }

            if sum > 100 {
                show_debug_message("yes");
            }

            test.foo = 1;
            test.bar = {
                a: 1,
                b: 2,
            };

            var i = 1_234;
            var j = new Foo();

            switch i {
                case 1_234: {}
                default: {}
            }

            var a = [1, 2, 3];
            print(a[@ 1]);
        "#;

        parse(
            ParseSettings {
                strict_semicolons: true,
                ..ParseSettings::compat()
            },
            SOURCE,
        )
        .unwrap();
    }
}
