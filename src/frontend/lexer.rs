use crate::frontend::error::{FrontendError, FrontendErrorKind};
use crate::frontend::token::{lookup_keyword, IntSuffix, Span, Token, TokenKind};

/// 词法分析器
pub struct Lexer {
    source: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
    byte_pos: usize,
    errors: Vec<FrontendError>,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
            byte_pos: 0,
            errors: Vec::new(),
        }
    }

    pub fn errors(&self) -> &[FrontendError] {
        &self.errors
    }

    /// 返回所有 token（包括 Error token）
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token();
            let is_eof = matches!(token.kind, TokenKind::Eof);
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        tokens
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace_and_comments();

        if self.is_eof() {
            return self.make_token(TokenKind::Eof);
        }

        let ch = self.current_char();

        // 下划线（通配符）
        if ch == '_' && !self.peek_char_next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            self.advance();
            return self.make_token(TokenKind::Underscore);
        }

        // 标识符或关键字
        if ch.is_ascii_alphabetic() || ch == '_' {
            return self.lex_ident_or_keyword();
        }

        // 数字字面量
        if ch.is_ascii_digit() {
            return self.lex_number();
        }

        // 字符和字符串
        if ch == '\'' {
            return self.lex_char();
        }
        if ch == '"' {
            return self.lex_string();
        }

        // 单字符运算符和分隔符
        match ch {
            '+' => {
                self.advance();
                self.make_token(TokenKind::Plus)
            }
            '-' => {
                self.advance();
                if self.peek_char() == Some('>') {
                    self.advance();
                    self.make_token(TokenKind::Arrow)
                } else {
                    self.make_token(TokenKind::Minus)
                }
            }
            '*' => {
                self.advance();
                self.make_token(TokenKind::Star)
            }
            '/' => {
                self.advance();
                self.make_token(TokenKind::Slash)
            }
            '%' => {
                self.advance();
                self.make_token(TokenKind::Percent)
            }
            '=' => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    self.make_token(TokenKind::EqEq)
                } else if self.peek_char() == Some('>') {
                    self.advance();
                    self.make_token(TokenKind::FatArrow)
                } else {
                    self.make_token(TokenKind::Eq)
                }
            }
            '!' => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    self.make_token(TokenKind::Ne)
                } else {
                    self.make_token(TokenKind::Bang)
                }
            }
            '<' => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    self.make_token(TokenKind::Le)
                } else {
                    self.make_token(TokenKind::Lt)
                }
            }
            '>' => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    self.make_token(TokenKind::Ge)
                } else {
                    self.make_token(TokenKind::Gt)
                }
            }
            '&' => {
                self.advance();
                if self.peek_char() == Some('&') {
                    self.advance();
                    self.make_token(TokenKind::AndAnd)
                } else {
                    self.make_token(TokenKind::And)
                }
            }
            '|' => {
                self.advance();
                if self.peek_char() == Some('|') {
                    self.advance();
                    self.make_token(TokenKind::OrOr)
                } else {
                    self.make_token(TokenKind::Or)
                }
            }
            '^' => {
                self.advance();
                self.make_token(TokenKind::Caret)
            }
            '#' => {
                self.advance();
                self.make_token(TokenKind::Hash)
            }
            '@' => {
                self.advance();
                self.make_token(TokenKind::At)
            }
            '(' => {
                self.advance();
                self.make_token(TokenKind::LParen)
            }
            ')' => {
                self.advance();
                self.make_token(TokenKind::RParen)
            }
            '{' => {
                self.advance();
                self.make_token(TokenKind::LBrace)
            }
            '}' => {
                self.advance();
                self.make_token(TokenKind::RBrace)
            }
            '[' => {
                self.advance();
                self.make_token(TokenKind::LBracket)
            }
            ']' => {
                self.advance();
                self.make_token(TokenKind::RBracket)
            }
            '.' => {
                self.advance();
                if self.peek_char() == Some('.') {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        self.make_token(TokenKind::DotDotEq)
                    } else {
                        self.make_token(TokenKind::DotDot)
                    }
                } else {
                    self.make_token(TokenKind::Dot)
                }
            }
            ',' => {
                self.advance();
                self.make_token(TokenKind::Comma)
            }
            ':' => {
                self.advance();
                if self.peek_char() == Some(':') {
                    self.advance();
                    self.make_token(TokenKind::ColonColon)
                } else {
                    self.make_token(TokenKind::Colon)
                }
            }
            ';' => {
                self.advance();
                self.make_token(TokenKind::Semicolon)
            }
            _ => {
                // 意外字符
                let span = self.current_span();
                self.errors.push(FrontendError::new(
                    FrontendErrorKind::UnexpectedChar(ch),
                    span,
                ));
                self.advance();
                Token::new(TokenKind::Error(format!("unexpected character: {ch}")), span)
            }
        }
    }

    // ─── 内部方法 ───

    fn make_token(&self, kind: TokenKind) -> Token {
        Token::new(kind, Span::new(self.line, self.column, self.byte_pos))
    }

    fn current_span(&self) -> Span {
        Span::new(self.line, self.column, self.byte_pos)
    }

    fn current_char(&self) -> char {
        self.source[self.pos]
    }

    /// 查看当前字符（不推进）
    fn peek_char(&self) -> Option<char> {
        self.source.get(self.pos).copied()
    }

    /// 查看下下个字符（用于注释/操作符两字符前看）
    fn peek_char_next(&self) -> Option<char> {
        self.source.get(self.pos + 1).copied()
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.source.len()
    }

    fn advance(&mut self) {
        if !self.is_eof() {
            let ch = self.source[self.pos];
            self.pos += 1;
            self.byte_pos += ch.len_utf8();
            if ch == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            if self.is_eof() {
                return;
            }
            let ch = self.current_char();
            if ch.is_whitespace() {
                self.advance();
            } else if ch == '/' && self.peek_char_next() == Some('/') {
                self.skip_line_comment();
            } else if ch == '/' && self.peek_char_next() == Some('*') {
                self.skip_block_comment();
            } else {
                return;
            }
        }
    }

    fn skip_line_comment(&mut self) {
        self.advance(); // skip second /
        while !self.is_eof() && self.current_char() != '\n' {
            self.advance();
        }
    }

    fn skip_block_comment(&mut self) {
        self.advance(); // skip *
        let mut depth = 1u32;
        while depth > 0 && !self.is_eof() {
            if self.current_char() == '/' && self.peek_char_next() == Some('*') {
                self.advance();
                self.advance();
                depth += 1;
            } else if self.current_char() == '*' && self.peek_char_next() == Some('/') {
                self.advance();
                self.advance();
                depth -= 1;
            } else {
                self.advance();
            }
        }
    }

    fn lex_ident_or_keyword(&mut self) -> Token {
        let start_span = self.current_span();
        let mut name = String::new();
        while !self.is_eof() {
            let ch = self.current_char();
            if ch.is_ascii_alphanumeric() || ch == '_' {
                name.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        // 检查是否为 stardust! 宏
        if name == "stardust" && self.peek_char() == Some('!') {
            self.advance(); // skip !
            return Token::new(TokenKind::Stardust, start_span);
        }

        // 检查关键字
        if let Some(kw) = lookup_keyword(&name) {
            Token::new(kw, start_span)
        } else {
            Token::new(TokenKind::Ident(name), start_span)
        }
    }

    fn lex_number(&mut self) -> Token {
        let start_span = self.current_span();
        let ch = self.current_char();

        // 检查 0x/0o/0b 前缀
        if ch == '0' && let Some(next) = self.peek_char_next() {
            match next {
                'x' | 'X' => return self.lex_hex_int(),
                'o' | 'O' => return self.lex_octal_int(),
                'b' | 'B' => return self.lex_binary_int(),
                _ => {}
            }
        }

        // 十进制：整数或浮点数
        let mut num_str = String::new();
        while !self.is_eof() && self.current_char().is_ascii_digit() {
            num_str.push(self.current_char());
            self.advance();
        }

        // 检查是否有小数点（浮点数）
        if self.peek_char() == Some('.') && self.peek_char_next().is_some_and(|c| c.is_ascii_digit()) {
            self.advance(); // skip .
            num_str.push('.');
            while !self.is_eof() && self.current_char().is_ascii_digit() {
                num_str.push(self.current_char());
                self.advance();
            }
            // 检查指数（pos 在 '.' 之后的数字部分末尾，或 '.' 的位置）
            if self.peek_char() == Some('e') || self.peek_char() == Some('E') {
                self.advance();
                num_str.push('e');
                if self.peek_char() == Some('+') || self.peek_char() == Some('-') {
                    num_str.push(self.current_char());
                    self.advance();
                }
                while !self.is_eof() && self.current_char().is_ascii_digit() {
                    num_str.push(self.current_char());
                    self.advance();
                }
            }
            match num_str.parse::<f64>() {
                Ok(v) => Token::new(TokenKind::FloatLiteral(v), start_span),
                Err(_) => {
                    self.errors.push(FrontendError::new(
                        FrontendErrorKind::InvalidNumber(num_str),
                        start_span,
                    ));
                    Token::new(TokenKind::Error("invalid float".to_string()), start_span)
                }
            }
        } else {
            // 整数，可能带后缀（i8/i16/i32/i64/u8..u64/f32/f64）
            let (suffix, is_float) = self.lex_int_suffix();
            if is_float {
                if let Ok(v) = num_str.parse::<f64>() {
                    return Token::new(TokenKind::FloatLiteral(v), start_span);
                }
            }
            match num_str.parse::<i64>() {
                Ok(v) => Token::new(TokenKind::IntLiteral(v, suffix), start_span),
                Err(_) => {
                    self.errors.push(FrontendError::new(
                        FrontendErrorKind::InvalidNumber(num_str),
                        start_span,
                    ));
                    Token::new(TokenKind::Error("invalid integer".to_string()), start_span)
                }
            }
        }
    }

    fn lex_hex_int(&mut self) -> Token {
        let start_span = self.current_span();
        self.advance(); // 0
        self.advance(); // x
        let mut num_str = String::new();
        while !self.is_eof() && self.current_char().is_ascii_hexdigit() {
            num_str.push(self.current_char());
            self.advance();
        }
        let (suffix, _) = self.lex_int_suffix();
        if num_str.is_empty() {
            self.errors.push(FrontendError::new(
                FrontendErrorKind::InvalidNumber("0x".to_string()),
                start_span,
            ));
            return Token::new(TokenKind::Error("empty hex literal".to_string()), start_span);
        }
        match i64::from_str_radix(&num_str, 16) {
            Ok(v) => Token::new(TokenKind::IntLiteral(v, suffix), start_span),
            Err(_) => Token::new(TokenKind::Error("invalid hex literal".to_string()), start_span),
        }
    }

    fn lex_octal_int(&mut self) -> Token {
        let start_span = self.current_span();
        self.advance(); // 0
        self.advance(); // o
        let mut num_str = String::new();
        while !self.is_eof() && matches!(self.current_char(), '0'..='7') {
            num_str.push(self.current_char());
            self.advance();
        }
        let (suffix, _) = self.lex_int_suffix();
        if num_str.is_empty() {
            self.errors.push(FrontendError::new(
                FrontendErrorKind::InvalidNumber("0o".to_string()),
                start_span,
            ));
            return Token::new(TokenKind::Error("empty octal literal".to_string()), start_span);
        }
        match i64::from_str_radix(&num_str, 8) {
            Ok(v) => Token::new(TokenKind::IntLiteral(v, suffix), start_span),
            Err(_) => Token::new(TokenKind::Error("invalid octal literal".to_string()), start_span),
        }
    }

    fn lex_binary_int(&mut self) -> Token {
        let start_span = self.current_span();
        self.advance(); // 0
        self.advance(); // b
        let mut num_str = String::new();
        while !self.is_eof() && matches!(self.current_char(), '0' | '1') {
            num_str.push(self.current_char());
            self.advance();
        }
        let (suffix, _) = self.lex_int_suffix();
        if num_str.is_empty() {
            self.errors.push(FrontendError::new(
                FrontendErrorKind::InvalidNumber("0b".to_string()),
                start_span,
            ));
            return Token::new(TokenKind::Error("empty binary literal".to_string()), start_span);
        }
        match i64::from_str_radix(&num_str, 2) {
            Ok(v) => Token::new(TokenKind::IntLiteral(v, suffix), start_span),
            Err(_) => Token::new(TokenKind::Error("invalid binary literal".to_string()), start_span),
        }
    }

    fn lex_int_suffix(&mut self) -> (IntSuffix, bool) {
        if self.is_eof() {
            return (IntSuffix::None, false);
        }
        let ch = self.current_char();
        match ch {
            'i' => {
                self.advance();
                // i8, i16, i32, i64, isize
                let mut s = String::from("i");
                while !self.is_eof() && self.current_char().is_ascii_alphanumeric() {
                    s.push(self.current_char());
                    self.advance();
                }
                (IntSuffix::parse(&s), false)
            }
            'u' => {
                self.advance();
                let mut s = String::from("u");
                while !self.is_eof() && self.current_char().is_ascii_alphanumeric() {
                    s.push(self.current_char());
                    self.advance();
                }
                (IntSuffix::parse(&s), false)
            }
            'f' => {
                self.advance();
                // f32, f64
                let mut s = String::from("f");
                while !self.is_eof() && self.current_char().is_ascii_alphanumeric() {
                    s.push(self.current_char());
                    self.advance();
                }
                match s.as_str() {
                    "f32" => (IntSuffix::I32, true),
                    "f64" => (IntSuffix::I64, true),
                    _ => (IntSuffix::None, false),
                }
            }
            _ => (IntSuffix::None, false),
        }
    }

    fn lex_char(&mut self) -> Token {
        let start_span = self.current_span();
        self.advance(); // skip opening '

        if self.is_eof() {
            self.errors.push(FrontendError::new(
                FrontendErrorKind::UnterminatedChar,
                start_span,
            ));
            return Token::new(TokenKind::Error("unterminated char".to_string()), start_span);
        }

        let ch_val = if self.current_char() == '\\' {
            self.advance();
            match self.lex_escape() {
                Ok(v) => v,
                Err(e) => {
                    self.errors.push(FrontendError::new(e, start_span));
                    return Token::new(TokenKind::Error("invalid char escape".to_string()), start_span);
                }
            }
        } else {
            let ch = self.current_char();
            self.advance();
            ch as u32
        };

        if self.is_eof() || self.current_char() != '\'' {
            self.errors.push(FrontendError::new(
                FrontendErrorKind::UnterminatedChar,
                start_span,
            ));
            return Token::new(TokenKind::Error("unterminated char".to_string()), start_span);
        }
        self.advance(); // skip closing '

        Token::new(TokenKind::CharLiteral(ch_val), start_span)
    }

    fn lex_string(&mut self) -> Token {
        let start_span = self.current_span();
        self.advance(); // skip opening "

        let mut s = String::new();
        loop {
            if self.is_eof() {
                self.errors.push(FrontendError::new(
                    FrontendErrorKind::UnterminatedString,
                    start_span,
                ));
                return Token::new(TokenKind::Error("unterminated string".to_string()), start_span);
            }
            let ch = self.current_char();
            if ch == '"' {
                self.advance(); // skip closing "
                break;
            }
            if ch == '\\' {
                self.advance();
                match self.lex_escape() {
                    Ok(val) => {
                        if let Some(c) = char::from_u32(val) {
                            s.push(c);
                        } else {
                            s.push('\u{FFFD}');
                        }
                    }
                    Err(e) => {
                        self.errors.push(FrontendError::new(e, start_span));
                    }
                }
            } else {
                s.push(ch);
                self.advance();
            }
        }

        Token::new(TokenKind::StringLiteral(s), start_span)
    }

    fn lex_escape(&mut self) -> Result<u32, FrontendErrorKind> {
        if self.is_eof() {
            return Err(FrontendErrorKind::InvalidCharEscape("unexpected EOF".to_string()));
        }
        match self.current_char() {
            'n' => { self.advance(); Ok('\n' as u32) }
            't' => { self.advance(); Ok('\t' as u32) }
            'r' => { self.advance(); Ok('\r' as u32) }
            '\\' => { self.advance(); Ok('\\' as u32) }
            '\'' => { self.advance(); Ok('\'' as u32) }
            '"' => { self.advance(); Ok('"' as u32) }
            '0' => { self.advance(); Ok('\0' as u32) }
            'u' => {
                self.advance();
                if self.current_char() != '{' {
                    return Err(FrontendErrorKind::InvalidEscapeSequence("u".to_string()));
                }
                self.advance(); // skip {
                let mut hex = String::new();
                while !self.is_eof() && self.current_char() != '}' {
                    hex.push(self.current_char());
                    self.advance();
                }
                if self.is_eof() {
                    return Err(FrontendErrorKind::InvalidEscapeSequence("u".to_string()));
                }
                self.advance(); // skip }
                u32::from_str_radix(&hex, 16)
                    .map_err(|_| FrontendErrorKind::InvalidCharEscape(hex))
            }
            'x' => {
                self.advance();
                let mut hex = String::new();
                for _ in 0..2 {
                    if !self.is_eof() && self.current_char().is_ascii_hexdigit() {
                        hex.push(self.current_char());
                        self.advance();
                    } else {
                        break;
                    }
                }
                if hex.is_empty() {
                    return Err(FrontendErrorKind::InvalidEscapeSequence("x".to_string()));
                }
                u32::from_str_radix(&hex, 16)
                    .map_err(|_| FrontendErrorKind::InvalidCharEscape(hex))
            }
            _ => {
                let ch = self.current_char();
                self.advance();
                Err(FrontendErrorKind::InvalidEscapeSequence(ch.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(source: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(source);
        lexer.tokenize()
    }

    fn check_tokens(source: &str, expected: &[TokenKind]) {
        let tokens = tokenize(source);
        let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind.clone()).collect();
        let kinds_no_eof: Vec<TokenKind> = kinds.into_iter().filter(|k| !matches!(k, TokenKind::Eof)).collect();
        assert_eq!(kinds_no_eof.len(), expected.len(), "token count mismatch: {kinds_no_eof:?}");
        for (i, (actual, exp)) in kinds_no_eof.iter().zip(expected.iter()).enumerate() {
            assert_eq!(actual, exp, "token[{i}] mismatch");
        }
    }

    #[test]
    fn test_keywords() {
        check_tokens("fn let mut if else while loop for in match return ret struct enum extern use unsafe as break continue",
            &[
                TokenKind::Fn, TokenKind::Let, TokenKind::Mut,
                TokenKind::If, TokenKind::Else, TokenKind::While,
                TokenKind::Loop, TokenKind::For, TokenKind::In,
                TokenKind::Match, TokenKind::Return, TokenKind::Ret,
                TokenKind::Struct, TokenKind::Enum, TokenKind::Extern,
                TokenKind::Use, TokenKind::Unsafe, TokenKind::As,
                TokenKind::Break, TokenKind::Continue,
            ]);
    }

    #[test]
    fn test_identifiers() {
        check_tokens("foo bar_baz _x42",
            &[
                TokenKind::Ident("foo".into()),
                TokenKind::Ident("bar_baz".into()),
                TokenKind::Ident("_x42".into()),
            ]);
    }

    #[test]
    fn test_integers() {
        check_tokens("0 42 0xFF 0o77 0b1010 100i32 200u64",
            &[
                TokenKind::IntLiteral(0, IntSuffix::None),
                TokenKind::IntLiteral(42, IntSuffix::None),
                TokenKind::IntLiteral(255, IntSuffix::None),
                TokenKind::IntLiteral(63, IntSuffix::None),
                TokenKind::IntLiteral(10, IntSuffix::None),
                TokenKind::IntLiteral(100, IntSuffix::I32),
                TokenKind::IntLiteral(200, IntSuffix::U64),
            ]);
    }

    #[test]
    fn test_floats() {
        check_tokens("3.14 1.5e10 0.5",
            &[
                TokenKind::FloatLiteral(3.14),
                TokenKind::FloatLiteral(1.5e10),
                TokenKind::FloatLiteral(0.5),
            ]);
    }

    #[test]
    fn test_strings() {
        check_tokens(r#""hello" "line\nbreak""#,
            &[
                TokenKind::StringLiteral("hello".into()),
                TokenKind::StringLiteral("line\nbreak".into()),
            ]);
    }

    #[test]
    fn test_chars() {
        let tokens = tokenize("'A' '\\n' '猫'");
        let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind.clone()).collect();
        assert!(matches!(&kinds[0], TokenKind::CharLiteral(65))); // 'A'
        assert!(matches!(&kinds[1], TokenKind::CharLiteral(10))); // '\n'
    }

    #[test]
    fn test_unknown_escape_is_error() {
        let mut lexer = Lexer::new(r"'\q'");
        let _ = lexer.tokenize();
        assert!(
            lexer
                .errors()
                .iter()
                .any(|e| matches!(e.kind, FrontendErrorKind::InvalidEscapeSequence(_))),
            "expected invalid escape error, got {:?}",
            lexer.errors()
        );

        let mut lexer = Lexer::new(r#""a\q""#);
        let _ = lexer.tokenize();
        assert!(
            lexer
                .errors()
                .iter()
                .any(|e| matches!(e.kind, FrontendErrorKind::InvalidEscapeSequence(_))),
            "expected invalid escape error in string, got {:?}",
            lexer.errors()
        );
    }

    #[test]
    fn test_operators() {
        check_tokens("+ - * / % = == != < > <= >= && || & | ^ !",
            &[
                TokenKind::Plus, TokenKind::Minus, TokenKind::Star,
                TokenKind::Slash, TokenKind::Percent, TokenKind::Eq,
                TokenKind::EqEq, TokenKind::Ne, TokenKind::Lt, TokenKind::Gt,
                TokenKind::Le, TokenKind::Ge, TokenKind::AndAnd, TokenKind::OrOr,
                TokenKind::And, TokenKind::Or, TokenKind::Caret, TokenKind::Bang,
            ]);
    }

    #[test]
    fn test_delimiters() {
        check_tokens("() {} [] , ; : :: -> => . .. ..=",
            &[
                TokenKind::LParen, TokenKind::RParen,
                TokenKind::LBrace, TokenKind::RBrace,
                TokenKind::LBracket, TokenKind::RBracket,
                TokenKind::Comma, TokenKind::Semicolon,
                TokenKind::Colon, TokenKind::ColonColon,
                TokenKind::Arrow, TokenKind::FatArrow,
                TokenKind::Dot, TokenKind::DotDot, TokenKind::DotDotEq,
            ]);
    }

    #[test]
    fn test_comments() {
        // 单行注释后跟代码
        check_tokens("// this is a comment\n42",
            &[TokenKind::IntLiteral(42, IntSuffix::None)]);

        // 块注释
        check_tokens("/* block */ 42",
            &[TokenKind::IntLiteral(42, IntSuffix::None)]);

        // 嵌套块注释
        check_tokens("/* outer /* inner */ still */ 42",
            &[TokenKind::IntLiteral(42, IntSuffix::None)]);
    }

    #[test]
    fn test_stardust_macro() {
        check_tokens("stardust! { Push(3) }",
            &[
                TokenKind::Stardust,
                TokenKind::LBrace,
                TokenKind::Ident("Push".into()),
                TokenKind::LParen,
                TokenKind::IntLiteral(3, IntSuffix::None),
                TokenKind::RParen,
                TokenKind::RBrace,
            ]);
    }

    #[test]
    fn test_hash() {
        check_tokens("#x",
            &[
                TokenKind::Hash,
                TokenKind::Ident("x".into()),
            ]);
    }
}
