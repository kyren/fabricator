use std::mem;

use arrayvec::ArrayVec;
use fabricator_vm::Span;
use thiserror::Error;

use crate::{
    string_interner::StringInterner,
    tokens::{Token, TokenKind},
};

#[derive(Debug, Error)]
pub enum LexErrorKind {
    #[error("expected matching '\"' for string")]
    UnfinishedString,
    #[error("invalid string escape char {0:?}")]
    InvalidStringEscape(char),
    #[error("unexpected character: {0:?}")]
    UnexpectedCharacter(char),
    #[error("no such # directive \"#{0}\"")]
    BadDirective(String),
}

#[derive(Debug, Error)]
#[error("{kind}")]
pub struct LexError {
    pub kind: LexErrorKind,
    pub span: Span,
}

pub struct Lexer<'a, S> {
    interner: S,
    source: &'a str,
    peek_buffer: ArrayVec<char, 3>,
    string_buffer: String,
    position: usize,
}

impl<'a, S> Lexer<'a, S>
where
    S: StringInterner,
{
    /// Lexes the provided source completely, placing lexed tokens into the provided buffer.
    ///
    /// Always pushes a single [`TokenKind::EndOfStream`] token at the end of the stream.
    pub fn tokenize(
        interner: S,
        source: &'a str,
        tokens: &mut Vec<Token<S::String>>,
    ) -> Result<(), LexError> {
        let mut this = Self::new(interner, source);
        loop {
            let token = this.read_token()?;
            let at_end = matches!(token.kind, TokenKind::EndOfStream);
            tokens.push(token);
            if at_end {
                break;
            }
        }
        Ok(())
    }

    pub fn new(interner: S, source: &'a str) -> Lexer<'a, S> {
        Lexer {
            interner,
            source,
            peek_buffer: ArrayVec::new(),
            string_buffer: String::new(),
            position: 0,
        }
    }

    /// Returns the current byte position in the source file.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Skips any leading whitespace before the next token in the stream.
    ///
    /// It is not necessary to call this explicitly, but doing so will make the current byte
    /// position accurate for the start of the next returned token.
    pub fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek(0) {
            let nc = self.peek(1);

            match (c, nc) {
                ('/', Some('/')) => {
                    self.advance(2);
                    // Read until end of line
                    while let Some(c) = self.peek(0) {
                        if is_newline(c) {
                            break;
                        } else {
                            self.advance(1);
                        }
                    }
                }
                ('/', Some('*')) => {
                    self.advance(2);
                    // read until '*/'
                    loop {
                        match (self.peek(0), self.peek(1)) {
                            (Some('*'), Some('/')) => {
                                self.advance(2);
                                break;
                            }
                            (None, _) => break,
                            _ => self.advance(1),
                        }
                    }
                }
                _ if c.is_ascii_whitespace() && !is_newline(c) => {
                    self.advance(1);
                }
                _ => break,
            }
        }
    }

    /// Read and return the next token in the stream.
    ///
    /// Returns `TokenKind::EndOfStream` once the token stream is finished.
    pub fn read_token(&mut self) -> Result<Token<S::String>, LexError> {
        self.skip_whitespace();

        let start_position = self.position;
        let kind = match (self.peek(0), self.peek(1), self.peek(2)) {
            (Some(c), n, _) if is_newline(c) => {
                self.advance(1);
                // If we have a second newline character immediately following the first one and
                // it is a *different* newline character, then lex the pair "\n\r" or "\r\n" as a
                // single newline.
                if n.is_some_and(|nc| is_newline(nc) && nc != c) {
                    self.advance(1);
                }
                TokenKind::Newline
            }
            (Some(c), _, _) if c.is_ascii_whitespace() => {
                unreachable!("whitespace should have been skipped")
            }
            (Some(c), Some(n), _) if c == '#' && is_identifier_start_char(n) => {
                self.advance(1);
                self.read_identifier();
                match self.string_buffer.as_str() {
                    "macro" => TokenKind::Macro,
                    _ => {
                        return Err(LexError {
                            kind: LexErrorKind::BadDirective(mem::take(&mut self.string_buffer)),
                            span: Span::new(start_position, self.position),
                        });
                    }
                }
            }
            (Some('?'), Some('?'), Some('=')) => {
                self.advance(3);
                TokenKind::DoubleQuestionMarkEqual
            }
            (Some('!'), Some('='), _) => {
                self.advance(2);
                TokenKind::BangEqual
            }
            (Some('='), Some('='), _) => {
                self.advance(2);
                TokenKind::DoubleEqual
            }
            (Some('+'), Some('='), _) => {
                self.advance(2);
                TokenKind::PlusEqual
            }
            (Some('-'), Some('='), _) => {
                self.advance(2);
                TokenKind::MinusEqual
            }
            (Some('*'), Some('='), _) => {
                self.advance(2);
                TokenKind::StarEqual
            }
            (Some('/'), Some('='), _) => {
                self.advance(2);
                TokenKind::SlashEqual
            }
            (Some('%'), Some('='), _) => {
                self.advance(2);
                TokenKind::PercentEqual
            }
            (Some('&'), Some('='), _) => {
                self.advance(2);
                TokenKind::AmpersandEqual
            }
            (Some('|'), Some('='), _) => {
                self.advance(2);
                TokenKind::PipeEqual
            }
            (Some('^'), Some('='), _) => {
                self.advance(2);
                TokenKind::CaretEqual
            }
            (Some('<'), Some('='), _) => {
                self.advance(2);
                TokenKind::LessEqual
            }
            (Some('>'), Some('='), _) => {
                self.advance(2);
                TokenKind::GreaterEqual
            }
            (Some('?'), Some('?'), _) => {
                self.advance(2);
                TokenKind::DoubleQuestionMark
            }
            (Some('+'), Some('+'), _) => {
                self.advance(2);
                TokenKind::DoublePlus
            }
            (Some('-'), Some('-'), _) => {
                self.advance(2);
                TokenKind::DoubleMinus
            }
            (Some('&'), Some('&'), _) => {
                self.advance(2);
                TokenKind::DoubleAmpersand
            }
            (Some('|'), Some('|'), _) => {
                self.advance(2);
                TokenKind::DoublePipe
            }
            (Some('^'), Some('^'), _) => {
                self.advance(2);
                TokenKind::DoubleCaret
            }
            (Some('<'), Some('<'), _) => {
                self.advance(2);
                TokenKind::DoubleLess
            }
            (Some('>'), Some('>'), _) => {
                self.advance(2);
                TokenKind::DoubleGreater
            }
            (Some('('), _, _) => {
                self.advance(1);
                TokenKind::LeftParen
            }
            (Some(')'), _, _) => {
                self.advance(1);
                TokenKind::RightParen
            }
            (Some('['), _, _) => {
                self.advance(1);
                TokenKind::LeftBracket
            }
            (Some(']'), _, _) => {
                self.advance(1);
                TokenKind::RightBracket
            }
            (Some('{'), _, _) => {
                self.advance(1);
                TokenKind::LeftBrace
            }
            (Some('}'), _, _) => {
                self.advance(1);
                TokenKind::RightBrace
            }
            (Some(':'), _, _) => {
                self.advance(1);
                TokenKind::Colon
            }
            (Some(';'), _, _) => {
                self.advance(1);
                TokenKind::SemiColon
            }
            (Some(','), _, _) => {
                self.advance(1);
                TokenKind::Comma
            }
            (Some('+'), _, _) => {
                self.advance(1);
                TokenKind::Plus
            }
            (Some('-'), _, _) => {
                self.advance(1);
                TokenKind::Minus
            }
            (Some('!'), _, _) => {
                self.advance(1);
                TokenKind::Bang
            }
            (Some('/'), _, _) => {
                self.advance(1);
                TokenKind::Slash
            }
            (Some('*'), _, _) => {
                self.advance(1);
                TokenKind::Star
            }
            (Some('%'), _, _) => {
                self.advance(1);
                TokenKind::Percent
            }
            (Some('&'), _, _) => {
                self.advance(1);
                TokenKind::Ampersand
            }
            (Some('|'), _, _) => {
                self.advance(1);
                TokenKind::Pipe
            }
            (Some('~'), _, _) => {
                self.advance(1);
                TokenKind::Tilde
            }
            (Some('^'), _, _) => {
                self.advance(1);
                TokenKind::Caret
            }
            (Some('?'), _, _) => {
                self.advance(1);
                TokenKind::QuestionMark
            }
            (Some('#'), _, _) => {
                self.advance(1);
                TokenKind::Octothorpe
            }
            (Some('@'), _, _) => {
                self.advance(1);
                TokenKind::AtSign
            }
            (Some('='), _, _) => {
                self.advance(1);
                TokenKind::Equal
            }
            (Some('<'), _, _) => {
                self.advance(1);
                TokenKind::Less
            }
            (Some('>'), _, _) => {
                self.advance(1);
                TokenKind::Greater
            }
            (Some('"'), _, _) => {
                self.read_string()?;
                TokenKind::String(self.interner.intern(self.string_buffer.as_str()))
            }
            (Some('$'), n, _) => {
                if n.is_some_and(|c| c.is_ascii_hexdigit()) {
                    self.read_number()
                } else {
                    self.advance(1);
                    TokenKind::Dollar
                }
            }
            (Some('.'), n, _) => {
                if n.is_some_and(|c| c.is_ascii_digit()) {
                    self.read_number()
                } else {
                    self.advance(1);
                    TokenKind::Dot
                }
            }
            (Some(c), _, _) if c.is_ascii_digit() => self.read_number(),
            (Some(c), _, _) if is_identifier_start_char(c) => {
                self.read_identifier();
                match self.string_buffer.as_str() {
                    "mod" => TokenKind::Mod,
                    "div" => TokenKind::Div,
                    "and" => TokenKind::And,
                    "or" => TokenKind::Or,
                    "xor" => TokenKind::Xor,
                    "enum" => TokenKind::Enum,
                    "function" => TokenKind::Function,
                    "closure" => TokenKind::Closure,
                    "constructor" => TokenKind::Constructor,
                    "var" => TokenKind::Var,
                    "let" => TokenKind::Let,
                    "static" => TokenKind::Static,
                    "globalvar" => TokenKind::GlobalVar,
                    "switch" => TokenKind::Switch,
                    "case" => TokenKind::Case,
                    "default" => TokenKind::Default,
                    "break" => TokenKind::Break,
                    "continue" => TokenKind::Continue,
                    "if" => TokenKind::If,
                    "else" => TokenKind::Else,
                    "for" => TokenKind::For,
                    "repeat" => TokenKind::Repeat,
                    "while" => TokenKind::While,
                    "with" => TokenKind::With,
                    "return" => TokenKind::Return,
                    "exit" => TokenKind::Exit,
                    "throw" => TokenKind::Throw,
                    "try" => TokenKind::Try,
                    "catch" => TokenKind::Catch,
                    "undefined" => TokenKind::Undefined,
                    "true" => TokenKind::True,
                    "false" => TokenKind::False,
                    "global" => TokenKind::Global,
                    "self" => TokenKind::This,
                    "other" => TokenKind::Other,
                    "new" => TokenKind::New,
                    "argument" => TokenKind::Argument,
                    "argument_count" => TokenKind::ArgumentCount,
                    id => TokenKind::Identifier(self.interner.intern(id)),
                }
            }
            (Some(c), _, _) => {
                return Err(LexError {
                    kind: LexErrorKind::UnexpectedCharacter(c),
                    span: Span::new(start_position, start_position + 1),
                });
            }
            (None, _, _) => TokenKind::EndOfStream,
        };

        Ok(Token {
            kind,
            span: Span::new(start_position, self.position),
        })
    }

    /// Read an identifier into the string buffer.
    fn read_identifier(&mut self) {
        let start = self.peek(0).unwrap();
        assert!(is_identifier_start_char(start));

        self.string_buffer.clear();
        self.string_buffer.push(start);
        self.advance(1);

        while let Some(c) = self.peek(0) {
            if !is_identifier_char(c) {
                break;
            }

            self.string_buffer.push(c);
            self.advance(1);
        }
    }

    /// Read a short (not multi-line) string literal into the string buffer.
    fn read_string(&mut self) -> Result<(), LexError> {
        assert!(self.peek(0).unwrap() == '"');
        self.advance(1);

        self.string_buffer.clear();

        loop {
            let c = if let Some(c) = self.peek(0) {
                c
            } else {
                return Err(LexError {
                    kind: LexErrorKind::UnfinishedString,
                    span: Span::empty(self.position),
                });
            };

            if is_newline(c) {
                return Err(LexError {
                    kind: LexErrorKind::UnfinishedString,
                    span: Span::empty(self.position),
                });
            }

            self.advance(1);
            if c == '\\' {
                match self.peek(0).ok_or(LexError {
                    kind: LexErrorKind::UnfinishedString,
                    span: Span::empty(self.position),
                })? {
                    'n' => {
                        self.advance(1);
                        self.string_buffer.push('\n');
                    }

                    'r' => {
                        self.advance(1);
                        self.string_buffer.push('\r');
                    }

                    't' => {
                        self.advance(1);
                        self.string_buffer.push('\t');
                    }

                    '\\' => {
                        self.advance(1);
                        self.string_buffer.push('\\');
                    }

                    '"' => {
                        self.advance(1);
                        self.string_buffer.push('"');
                    }

                    c => {
                        return Err(LexError {
                            kind: LexErrorKind::InvalidStringEscape(c),
                            span: Span::new(self.position - 1, self.position + 1),
                        });
                    }
                }
            } else if c == '"' {
                break;
            } else {
                self.string_buffer.push(c);
            }
        }

        Ok(())
    }

    // Reads a hex or decimal integer or floating point number. Allows decimal integers (123), hex
    // integers (0xdeadbeef), hex integers prefixed with a '$' ($deadbeef), and decimal floating
    // point with optional exponent and exponent sign (3.21e+1).
    fn read_number(&mut self) -> TokenKind<S::String> {
        self.string_buffer.clear();

        let mut is_hex = false;
        let mut has_dollar_prefix = false;
        match (self.peek(0), self.peek(1)) {
            (Some('$'), _) => {
                is_hex = true;
                has_dollar_prefix = true;
                self.string_buffer.push('$');
                self.advance(1);
            }
            (Some('0'), Some(x)) if x.eq_ignore_ascii_case(&'x') => {
                is_hex = true;
                self.string_buffer.extend(['0', x]);
                self.advance(2);
            }
            _ => {}
        }

        let mut has_radix = false;
        while let Some(c) = self.peek(0) {
            if !is_hex && c == '.' && !has_radix {
                self.string_buffer.push('.');
                has_radix = true;
                self.advance(1);
            } else if c == '_'
                || (!is_hex && c.is_ascii_digit())
                || (is_hex && c.is_ascii_hexdigit())
            {
                self.string_buffer.push(c);
                self.advance(1);
            } else {
                break;
            }
        }

        let mut has_exp = false;
        if !is_hex {
            if let Some(exp_begin) = self.peek(0) {
                if (is_hex && exp_begin.eq_ignore_ascii_case(&'p'))
                    || (!is_hex && exp_begin.eq_ignore_ascii_case(&'e'))
                {
                    self.string_buffer.push(exp_begin);
                    has_exp = true;
                    self.advance(1);

                    if let Some(sign) = self.peek(0) {
                        if sign == '+' || sign == '-' {
                            self.string_buffer.push(sign);
                            self.advance(1);
                        }
                    }

                    while let Some(c) = self.peek(0) {
                        if c.is_ascii_digit() {
                            self.string_buffer.push(c);
                            self.advance(1);
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        let s = self.interner.intern(&self.string_buffer);
        if !has_exp && !has_radix {
            if is_hex {
                if has_dollar_prefix {
                    TokenKind::DollarHexInteger(s)
                } else {
                    TokenKind::HexInteger(s)
                }
            } else {
                assert!(!has_dollar_prefix);
                TokenKind::Integer(s)
            }
        } else {
            assert!(!is_hex && !has_dollar_prefix);
            TokenKind::Float(s)
        }
    }

    // Peek to the `n`th character ahead in the character stream.
    fn peek(&mut self, n: usize) -> Option<char> {
        while self.peek_buffer.len() <= n {
            let mut chars = self.source.chars();
            if let Some(c) = chars.next() {
                self.peek_buffer.push(c);
                self.source = chars.as_str();
            } else {
                break;
            }
        }

        self.peek_buffer.get(n).copied()
    }

    /// Advance the character stream `n` characters.
    ///
    /// # Panics
    ///
    /// Panics if this would advance over characters that have not been observed with `Lexer::peek`.
    fn advance(&mut self, n: usize) {
        assert!(
            n <= self.peek_buffer.len(),
            "cannot advance over un-peeked characters"
        );
        self.position += n;
        self.peek_buffer.drain(0..n);
    }
}

fn is_newline(c: char) -> bool {
    c == '\n' || c == '\r'
}

fn is_identifier_start_char(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(source: &str) -> Result<Vec<TokenKind<String>>, LexError> {
        struct SimpleInterner;

        impl StringInterner for SimpleInterner {
            type String = String;

            fn intern(&mut self, s: &str) -> Self::String {
                s.to_owned()
            }
        }

        let mut tokens = Vec::new();
        let mut lexer = Lexer::new(SimpleInterner, source);
        loop {
            let token = lexer.read_token()?;
            match &token.kind {
                TokenKind::Newline => {}
                TokenKind::EndOfStream => break,
                _ => {
                    tokens.push(token.kind);
                }
            }
        }

        Ok(tokens)
    }

    #[test]
    fn test_lexer() {
        const SOURCE: &str = r#"
            // Line comment
            var sum = 0;
            for (var i = 0; i < 1000000; ++i) {
                /*
                    Multiline comment
                */
                sum += i;
            }

            var hex = 0xdeadbeef;
            var flt = 3.21e+1;
        "#;

        assert_eq!(
            lex(SOURCE).unwrap(),
            vec![
                TokenKind::Var,
                TokenKind::Identifier("sum"),
                TokenKind::Equal,
                TokenKind::Integer("0"),
                TokenKind::SemiColon,
                TokenKind::For,
                TokenKind::LeftParen,
                TokenKind::Var,
                TokenKind::Identifier("i"),
                TokenKind::Equal,
                TokenKind::Integer("0"),
                TokenKind::SemiColon,
                TokenKind::Identifier("i"),
                TokenKind::Less,
                TokenKind::Integer("1000000"),
                TokenKind::SemiColon,
                TokenKind::DoublePlus,
                TokenKind::Identifier("i"),
                TokenKind::RightParen,
                TokenKind::LeftBrace,
                TokenKind::Identifier("sum"),
                TokenKind::PlusEqual,
                TokenKind::Identifier("i"),
                TokenKind::SemiColon,
                TokenKind::RightBrace,
                TokenKind::Var,
                TokenKind::Identifier("hex"),
                TokenKind::Equal,
                TokenKind::HexInteger("0xdeadbeef"),
                TokenKind::SemiColon,
                TokenKind::Var,
                TokenKind::Identifier("flt"),
                TokenKind::Equal,
                TokenKind::Float("3.21e+1"),
                TokenKind::SemiColon,
            ]
        );
    }
}
