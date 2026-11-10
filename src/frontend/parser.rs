use crate::frontend::ast::*;
use crate::frontend::error::{FrontendError, FrontendErrorKind};
use crate::frontend::token::{IntSuffix as TokenIntSuffix, Span, Token, TokenKind};

/// 语法解析器 — 递归下降 + Pratt 表达式解析
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    errors: Vec<FrontendError>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0, errors: Vec::new() }
    }

    pub fn errors(&self) -> &[FrontendError] {
        &self.errors
    }

    // ─── 顶层 ───

    pub fn parse_program(&mut self) -> Program {
        let mut decls = Vec::new();
        while !self.is_eof() {
            match self.parse_decl() {
                Ok(decl) => decls.push(decl),
                Err(e) => {
                    self.errors.push(e);
                    self.sync_to_decl();
                }
            }
        }
        Program { decls }
    }

    fn parse_decl(&mut self) -> Result<Decl, FrontendError> {
        let _attrs = self.parse_attributes();

        match self.peek() {
            TokenKind::Fn => self.parse_fn().map(Decl::Fn),
            TokenKind::Struct => self.parse_struct().map(Decl::Struct),
            TokenKind::Enum => self.parse_enum().map(Decl::Enum),
            TokenKind::Extern => self.parse_extern_block(),
            TokenKind::Use => self.parse_use_decl(),
            _ => {
                let token = self.advance();
                Err(FrontendError::new(
                    FrontendErrorKind::UnexpectedToken {
                        expected: "declaration (fn, struct, enum, extern, use)".to_string(),
                        found: token.clone(),
                    },
                    token.span,
                ))
            }
        }
    }

    // ─── 函数声明 ───

    fn parse_fn(&mut self) -> Result<FnDecl, FrontendError> {
        let fn_span = self.expect(TokenKind::Fn)?.span;
        let name = self.expect_ident("function name")?;

        self.expect(TokenKind::LParen)?;
        let params = self.parse_fn_params()?;
        self.expect(TokenKind::RParen)?;

        let return_type = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = if self.check(TokenKind::LBrace) {
            Some(self.parse_block()?)
        } else if self.check(TokenKind::Semicolon) {
            self.advance();
            None
        } else {
            let token = self.peek_token().clone();
            return Err(FrontendError::new(
                FrontendErrorKind::UnexpectedToken {
                    expected: "{ or ;".to_string(),
                    found: token,
                },
                fn_span,
            ));
        };

        Ok(FnDecl {
            name,
            params,
            return_type,
            body,
            attrs: vec![],
            span: fn_span,
        })
    }

    fn parse_fn_params(&mut self) -> Result<Vec<FnParam>, FrontendError> {
        let mut params = Vec::new();
        if self.check(TokenKind::RParen) {
            return Ok(params);
        }
        loop {
            let name = self.expect_ident("parameter name")?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let span = self.current_span();
            params.push(FnParam { name, ty, span });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        Ok(params)
    }

    // ─── 结构体声明 ───

    fn parse_struct(&mut self) -> Result<StructDecl, FrontendError> {
        let struct_span = self.expect(TokenKind::Struct)?.span;
        let name = self.expect_ident("struct name")?;
        self.expect(TokenKind::LBrace)?;
        let fields = self.parse_struct_fields()?;
        self.expect(TokenKind::RBrace)?;
        let _ = self.eat(TokenKind::Semicolon);

        Ok(StructDecl { name, fields, span: struct_span })
    }

    fn parse_struct_fields(&mut self) -> Result<Vec<StructField>, FrontendError> {
        let mut fields = Vec::new();
        if self.check(TokenKind::RBrace) {
            return Ok(fields);
        }
        loop {
            let name = self.expect_ident("field name")?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let span = self.current_span();
            fields.push(StructField { name, ty, span });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        Ok(fields)
    }

    // ─── 枚举声明 ───

    fn parse_enum(&mut self) -> Result<EnumDecl, FrontendError> {
        let enum_span = self.expect(TokenKind::Enum)?.span;
        let name = self.expect_ident("enum name")?;
        self.expect(TokenKind::LBrace)?;
        let variants = self.parse_enum_variants()?;
        self.expect(TokenKind::RBrace)?;
        let _ = self.eat(TokenKind::Semicolon);

        Ok(EnumDecl { name, variants, span: enum_span })
    }

    fn parse_enum_variants(&mut self) -> Result<Vec<EnumVariant>, FrontendError> {
        let mut variants = Vec::new();
        if self.check(TokenKind::RBrace) {
            return Ok(variants);
        }
        loop {
            let name = self.expect_ident("variant name")?;
            let span = self.current_span();
            let data = if self.eat(TokenKind::LParen) {
                let first_ty = self.parse_type()?;
                if self.eat(TokenKind::Comma) {
                    // 多类型元组: (u8, u8, u8)
                    let mut types = vec![first_ty];
                    loop {
                        types.push(self.parse_type()?);
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    Some(Type::Tuple(types, span))
                } else {
                    self.expect(TokenKind::RParen)?;
                    Some(first_ty)
                }
            } else {
                None
            };
            variants.push(EnumVariant { name, data, span });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        Ok(variants)
    }

    // ─── extern 块 ───

    fn parse_extern_block(&mut self) -> Result<Decl, FrontendError> {
        let ext_span = self.expect(TokenKind::Extern)?.span;
        let abi = self.expect_string("ABI string (e.g. \"stardust\")")?;
        self.expect(TokenKind::LBrace)?;

        let mut items = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_eof() {
            let _attrs = self.parse_attributes();
            if self.check(TokenKind::Fn) {
                let fn_decl = self.parse_fn()?;
                items.push(ExternItem::Fn(fn_decl));
            } else {
                let token = self.advance();
                self.errors.push(FrontendError::new(
                    FrontendErrorKind::UnexpectedToken {
                        expected: "fn in extern block".to_string(),
                        found: token,
                    },
                    ext_span,
                ));
                self.sync_to_decl();
            }
        }
        self.expect(TokenKind::RBrace)?;

        Ok(Decl::Extern(ExternBlock { abi, items, span: ext_span }))
    }

    // ─── use 声明 ───

    fn parse_use_decl(&mut self) -> Result<Decl, FrontendError> {
        let use_span = self.expect(TokenKind::Use)?.span;
        let mut path = Vec::new();
        loop {
            let seg = self.expect_ident("path segment")?;
            path.push(seg);
            if !self.eat(TokenKind::ColonColon) {
                break;
            }
        }
        self.expect(TokenKind::Semicolon)?;
        Ok(Decl::Use(path, use_span))
    }

    // ─── 语句 ───

    fn parse_stmt(&mut self) -> Result<Stmt, FrontendError> {
        match self.peek() {
            TokenKind::Let => self.parse_let_stmt(),
            TokenKind::Semicolon => {
                self.advance();
                self.parse_stmt()
            }
            _ => {
                let expr = self.parse_expr(0)?;
                if self.eat(TokenKind::Eq) && matches!(expr, Expr::Var(_, _)) {
                    let value = self.parse_expr(0)?;
                    let span = self.current_span();
                    self.expect_semicolon()?;
                    Ok(Stmt::Assign { target: expr, value, span })
                } else {
                    let span = self.current_span();
                    self.expect_semicolon()?;
                    Ok(Stmt::Expr(expr, span))
                }
            }
        }
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt, FrontendError> {
        let let_span = self.expect(TokenKind::Let)?.span;
        let mutable = self.eat(TokenKind::Mut);
        let pattern = self.parse_pattern()?;

        let ty = if self.eat(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let init = if self.eat(TokenKind::Eq) {
            Some(self.parse_expr(0)?)
        } else {
            None
        };

        self.expect_semicolon()?;

        Ok(Stmt::Let { pattern, ty, init, mutable, span: let_span })
    }

    // ─── 表达式 ───

    /// Pratt 表达式解析
    pub fn parse_expr(&mut self, min_prec: u8) -> Result<Expr, FrontendError> {
        let mut lhs = self.parse_expr_prefix()?;

        loop {
            let tk = self.peek();
            if matches!(tk, TokenKind::Eof | TokenKind::RBrace | TokenKind::Semicolon | TokenKind::RParen | TokenKind::RBracket | TokenKind::Comma | TokenKind::FatArrow) {
                break;
            }

            // 后缀操作
            match tk {
                TokenKind::LParen => {
                    lhs = self.parse_call(lhs)?;
                    continue;
                }
                TokenKind::Dot => {
                    self.advance();
                    let field = self.expect_ident("field name")?;
                    lhs = Expr::FieldAccess(Box::new(lhs), field, self.pos_span());
                    continue;
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr(0)?;
                    let span = self.expect(TokenKind::RBracket)?.span;
                    lhs = Expr::Index(Box::new(lhs), Box::new(index), span);
                    continue;
                }
                TokenKind::ColonColon => {
                    // 枚举变体或模块路径: A::B、A::B::C、A::B::C(payload)
                    let path_prefix = match &lhs {
                        Expr::Var(n, _) => Some(vec![n.clone()]),
                        Expr::EnumLit { name, variant, data: None, .. } => {
                            let mut path = name.clone();
                            path.push(variant.clone());
                            Some(path)
                        }
                        _ => None,
                    };
                    if let Some(path) = path_prefix {
                        self.advance();
                        let segment = self.expect_ident("path segment")?;
                        let span = self.pos_span();
                        let data = if self.check(TokenKind::LParen) {
                            self.advance(); // skip (
                            let data = if !self.check(TokenKind::RParen) {
                                Some(Box::new(self.parse_expr(0)?))
                            } else {
                                None
                            };
                            self.expect(TokenKind::RParen)?;
                            data
                        } else {
                            None
                        };
                        lhs = Expr::EnumLit {
                            name: path,
                            variant: segment,
                            data,
                            span,
                        };
                        continue;
                    }
                }
                TokenKind::LBrace if matches!(lhs, Expr::Var(_, _)) => {
                    // 结构体构造: Point { x: 1, y: 2 }
                    if let Expr::Var(n, _) = lhs {
                        let fields = self.parse_struct_literal_fields()?;
                        let span = self.pos_span();
                        lhs = Expr::StructLit { name: vec![n], fields, span };
                        continue;
                    }
                }
                _ => {}
            }

            // 中缀操作
            if let Some(op) = BinOp::from_token_kind(&tk) {
                let prec = op.precedence();
                if prec < min_prec {
                    break;
                }
                let span = self.advance().span;
                let rhs_prec = if op.is_right_associative() { prec } else { prec + 1 };
                let rhs = self.parse_expr(rhs_prec)?;
                lhs = Expr::Binary(Box::new(lhs), op, Box::new(rhs), span);
            } else {
                break;
            }
        }

        Ok(lhs)
    }

    /// 前缀表达式
    fn parse_expr_prefix(&mut self) -> Result<Expr, FrontendError> {
        let tk = self.peek();
        let span = self.peek_span();

        match tk {
            // 字面量
            TokenKind::IntLiteral(v, suffix) => {
                self.advance();
                let ast_suffix = convert_suffix(&suffix);
                Ok(Expr::Lit(Literal::Int(v, ast_suffix), span))
            }
            TokenKind::FloatLiteral(v) => {
                self.advance();
                Ok(Expr::Lit(Literal::Float(v), span))
            }
            TokenKind::BoolLiteral(b) => {
                self.advance();
                Ok(Expr::Lit(Literal::Bool(b), span))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::Lit(Literal::Bool(true), span))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::Lit(Literal::Bool(false), span))
            }
            TokenKind::CharLiteral(c) => {
                self.advance();
                Ok(Expr::Lit(Literal::Char(c), span))
            }
            TokenKind::StringLiteral(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expr::Lit(Literal::String(s), span))
            }

            // 标识符
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                // vec![...] 宏：目前与数组字面量语义一致，直接降级为 Expr::Array。
                if name == "vec"
                    && self.check(TokenKind::Bang)
                    && self.tokens.get(self.pos + 1).is_some_and(|t| {
                        matches!(t.kind, TokenKind::LBracket)
                    })
                {
                    self.advance(); // !
                    return self.parse_bracket_expr(span);
                }
                Ok(Expr::Var(name, span))
            }

            // #id
            TokenKind::Hash => {
                self.advance();
                let name = self.expect_ident("identifier after #")?;
                Ok(Expr::IdRef(name, span))
            }

            // 一元操作符
            TokenKind::Minus => {
                self.advance();
                let expr = self.parse_expr_prefix()?;
                Ok(Expr::Unary(UnOp::Neg, Box::new(expr), span))
            }
            TokenKind::Bang => {
                self.advance();
                let expr = self.parse_expr_prefix()?;
                Ok(Expr::Unary(UnOp::Not, Box::new(expr), span))
            }
            TokenKind::Star => {
                self.advance();
                let expr = self.parse_expr_prefix()?;
                Ok(Expr::Deref(Box::new(expr), span))
            }

            // & 引用
            TokenKind::And => {
                self.advance();
                let mutable = self.eat(TokenKind::Mut);
                let expr = self.parse_expr_prefix()?;
                Ok(Expr::Ref(Box::new(expr), mutable, span))
            }

            // 分组
            TokenKind::LParen => {
                self.advance();
                if self.eat(TokenKind::RParen) {
                    return Ok(Expr::Lit(Literal::Unit, span));
                }
                let expr = self.parse_expr(0)?;
                if self.eat(TokenKind::Comma) {
                    let mut elems = vec![expr];
                    loop {
                        if self.check(TokenKind::RParen) {
                            break;
                        }
                        let e = self.parse_expr(0)?;
                        elems.push(e);
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                    let end_span = self.expect(TokenKind::RParen)?.span;
                    Ok(Expr::Tuple(elems, end_span))
                } else {
                    self.expect(TokenKind::RParen)?;
                    Ok(expr)
                }
            }

            // 块
            TokenKind::LBrace => {
                let block = self.parse_block()?;
                Ok(Expr::Block(block, span))
            }

            // 数组
            TokenKind::LBracket => self.parse_bracket_expr(span),

            // 控制流
            TokenKind::If => self.parse_if_expr(),
            TokenKind::Match => self.parse_match_expr(),
            TokenKind::Loop => {
                self.advance();
                let body = self.parse_block()?;
                Ok(Expr::Loop(body, span))
            }
            TokenKind::While => {
                self.advance();
                let cond = self.parse_expr(0)?;
                let body = self.parse_block()?;
                Ok(Expr::While(Box::new(cond), body, span))
            }
            TokenKind::For => {
                self.advance();
                let pattern = self.parse_pattern()?;
                self.expect(TokenKind::In)?;
                let iterable = self.parse_expr(0)?;
                let body = self.parse_block()?;
                Ok(Expr::For { pattern, iterable: Box::new(iterable), body, span })
            }
            TokenKind::Return => {
                self.advance();
                let mut values = Vec::new();
                if !self.check(TokenKind::Semicolon) && !self.check(TokenKind::RBrace) {
                    loop {
                        values.push(self.parse_expr(0)?);
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                Ok(Expr::Return(values, span))
            }
            TokenKind::Ret => {
                self.advance();
                let n = self.parse_expr(0)?;
                if let Expr::Lit(Literal::Int(v, _), _) = &n {
                    Ok(Expr::Ret(*v as i32, span))
                } else {
                    Ok(Expr::Ret(0, span))
                }
            }
            TokenKind::Break => {
                self.advance();
                let value = if !self.check(TokenKind::Semicolon) && !self.check(TokenKind::RBrace) {
                    Some(Box::new(self.parse_expr(0)?))
                } else {
                    None
                };
                Ok(Expr::Break(value, span))
            }
            TokenKind::Continue => {
                self.advance();
                Ok(Expr::Continue(span))
            }
            TokenKind::Unsafe => {
                self.advance();
                let block = self.parse_block()?;
                Ok(Expr::Unsafe(block, span))
            }
            TokenKind::Stardust => {
                self.advance();
                self.expect(TokenKind::LBrace)?;
                let insts = self.parse_stardust_insts()?;
                self.expect(TokenKind::RBrace)?;
                Ok(Expr::StardustMacro(insts, span))
            }

            _ => {
                let token = self.advance();
                Err(FrontendError::new(
                    FrontendErrorKind::UnexpectedToken {
                        expected: "expression".to_string(),
                        found: token,
                    },
                    span,
                ))
            }
        }
    }

    /// 解析 `[...]` 数组字面量（当前 token 必须是 `[`）。
    /// 同时支持 `[a, b, c]`、`[a; n]` 和空数组 `[]`。
    fn parse_bracket_expr(&mut self, span: Span) -> Result<Expr, FrontendError> {
        self.expect(TokenKind::LBracket)?;
        if self.eat(TokenKind::RBracket) {
            return Ok(Expr::Array(vec![], span));
        }
        let first = self.parse_expr(0)?;
        if self.eat(TokenKind::Semicolon) {
            let count = self.parse_expr(0)?;
            self.expect(TokenKind::RBracket)?;
            Ok(Expr::Repeat(Box::new(first), Box::new(count), span))
        } else if self.eat(TokenKind::Comma) {
            let mut elems = vec![first];
            loop {
                if self.check(TokenKind::RBracket) {
                    break;
                }
                let e = self.parse_expr(0)?;
                elems.push(e);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RBracket)?;
            Ok(Expr::Array(elems, span))
        } else {
            self.expect(TokenKind::RBracket)?;
            Ok(Expr::Array(vec![first], span))
        }
    }

    fn parse_if_expr(&mut self) -> Result<Expr, FrontendError> {
        let span = self.advance().span;
        let cond = self.parse_expr(0)?;
        let then_block = self.parse_block()?;
        let else_block = if self.eat(TokenKind::Else) {
            if self.check(TokenKind::If) {
                let elif = self.parse_if_expr()?;
                Some(Block {
                    stmts: vec![Stmt::Expr(elif, span)],
                    tail: None,
                    span,
                })
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };
        Ok(Expr::If(Box::new(cond), then_block, else_block, span))
    }

    fn parse_match_expr(&mut self) -> Result<Expr, FrontendError> {
        let span = self.advance().span;
        // 使用 parse_expr_prefix 避免 suffix 循环将 { 误解为 struct literal
        let scrutinee = self.parse_expr_prefix()?;
        self.expect(TokenKind::LBrace)?;
        let arms = self.parse_match_arms()?;
        self.expect(TokenKind::RBrace)?;
        Ok(Expr::Match(Box::new(scrutinee), arms, span))
    }

    /// 解析函数调用
    fn parse_call(&mut self, func: Expr) -> Result<Expr, FrontendError> {
        let paren_span = self.expect(TokenKind::LParen)?.span;

        if self.eat(TokenKind::RParen) {
            return Ok(Expr::Call {
                func: Box::new(func),
                args: vec![],
                stack_pass: true,
                span: paren_span,
            });
        }

        let mut args = Vec::new();
        let mut has_named = false;
        let mut has_positional = false;

        loop {
            // 检查命名参数: name:
            if let TokenKind::Ident(_) = self.peek() {
                let saved_pos = self.pos;
                let name_token = self.advance();
                if let TokenKind::Ident(name) = &name_token.kind {
                    if self.eat(TokenKind::Colon) {
                        let value = self.parse_expr(0)?;
                        args.push(CallArg::Named(name.clone(), value, name_token.span));
                        has_named = true;
                    } else {
                        // 回退
                        self.pos = saved_pos;
                        args.push(CallArg::Pos(self.parse_expr(0)?, self.current_span()));
                        has_positional = true;
                    }
                }
            } else {
                args.push(CallArg::Pos(self.parse_expr(0)?, self.current_span()));
                has_positional = true;
            }

            if !self.eat(TokenKind::Comma) {
                break;
            }
        }

        if has_named && has_positional {
            self.errors.push(FrontendError::new(
                FrontendErrorKind::MixedCallArgs(paren_span),
                paren_span,
            ));
        }

        self.expect(TokenKind::RParen)?;

        Ok(Expr::Call {
            func: Box::new(func),
            args,
            stack_pass: false,
            span: paren_span,
        })
    }

    // ─── 块 ───

    fn parse_block(&mut self) -> Result<Block, FrontendError> {
        let open_span = self.expect(TokenKind::LBrace)?.span;
        let mut stmts = Vec::new();
        let mut tail: Option<Box<Expr>> = None;

        while !self.check(TokenKind::RBrace) && !self.is_eof() {
            if self.check(TokenKind::Let) || self.check(TokenKind::Semicolon) {
                stmts.push(self.parse_stmt()?);
            } else {
                // 块状表达式（块/if/match/循环）作为语句时到此结束，
                // 避免把下一条语句开头的 `*`、`-`、`&` 等误当成
                // 二元运算符继续解析（例如 `{ ... }\n*p = 4;`）。
                let block_like = matches!(
                    self.peek(),
                    TokenKind::LBrace
                        | TokenKind::If
                        | TokenKind::Match
                        | TokenKind::Loop
                        | TokenKind::While
                        | TokenKind::For
                        | TokenKind::Unsafe
                );
                let expr = if block_like {
                    self.parse_expr_prefix()?
                } else {
                    self.parse_expr(0)?
                };
                if self.check(TokenKind::RBrace) {
                    if tail.is_none() {
                        tail = Some(Box::new(expr));
                    } else {
                        stmts.push(Stmt::Expr(expr, self.current_span()));
                    }
                    break;
                } else if self.eat(TokenKind::Semicolon) {
                    stmts.push(Stmt::Expr(expr, self.current_span()));
                } else if self.eat(TokenKind::Eq) {
                    let value = self.parse_expr(0)?;
                    self.expect_semicolon()?;
                    stmts.push(Stmt::Assign { target: expr, value, span: self.current_span() });
                } else {
                    stmts.push(Stmt::Expr(expr, self.current_span()));
                }
            }
        }

        if !self.check(TokenKind::RBrace) {
            return Err(FrontendError::new(
                FrontendErrorKind::UnexpectedEof { expected: "}".to_string() },
                open_span,
            ));
        }
        self.advance(); // skip }

        Ok(Block { stmts, tail, span: open_span })
    }

    // ─── 模式 ───

    fn parse_pattern(&mut self) -> Result<Pattern, FrontendError> {
        match self.peek() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                let span = self.peek_span();
                self.advance();

                // 结构体模式: Name { ... }
                if self.check(TokenKind::LBrace) {
                    self.advance();
                    let mut fields = Vec::new();
                    let mut rest = false;
                    loop {
                        if self.check(TokenKind::RBrace) {
                            break;
                        }
                        if self.eat(TokenKind::DotDot) {
                            rest = true;
                            if !self.check(TokenKind::RBrace) {
                                self.expect(TokenKind::Comma)?;
                            }
                            break;
                        }
                        let fname = self.expect_ident("field name")?;
                        let subpat = if self.eat(TokenKind::Colon) {
                            Some(self.parse_pattern()?)
                        } else {
                            None
                        };
                        fields.push((fname, subpat));
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(TokenKind::RBrace)?;
                    Ok(Pattern::Struct { name: vec![name], fields, rest, span })
                } else if self.check(TokenKind::ColonColon) {
                    let mut path = vec![name];
                    while self.eat(TokenKind::ColonColon) {
                        path.push(self.expect_ident("path segment")?);
                    }
                    let variant = path.pop().unwrap();
                    let inner = if self.eat(TokenKind::LParen) {
                        let inner = Some(Box::new(self.parse_pattern()?));
                        self.expect(TokenKind::RParen)?;
                        inner
                    } else {
                        None
                    };
                    Ok(Pattern::Enum { name: path, variant, inner, span })
                } else {
                    Ok(Pattern::Var(name, span))
                }
            }
            TokenKind::Underscore => {
                let span = self.peek_span();
                self.advance();
                Ok(Pattern::Wildcard(span))
            }
            TokenKind::And => {
                let span = self.peek_span();
                self.advance();
                let mutable = self.eat(TokenKind::Mut);
                let inner = self.parse_pattern()?;
                Ok(Pattern::Ref(Box::new(inner), mutable, span))
            }
            TokenKind::Mut => {
                self.advance();
                self.parse_pattern() // mut 修饰在 let 语句层面处理
            }
            TokenKind::LParen => {
                self.advance();
                if self.eat(TokenKind::RParen) {
                    return Ok(Pattern::Tuple(vec![], None, self.current_span()));
                }
                let mut elems = Vec::new();
                let mut rest = None;
                loop {
                    if self.check(TokenKind::RParen) {
                        break;
                    }
                    if self.eat(TokenKind::DotDot) {
                        rest = Some(self.current_span());
                        if !self.check(TokenKind::RParen) {
                            self.expect(TokenKind::Comma)?;
                        }
                        break;
                    }
                    elems.push(self.parse_pattern()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                Ok(Pattern::Tuple(elems, rest, self.current_span()))
            }
            TokenKind::IntLiteral(v, _) => {
                let span = self.peek_span();
                self.advance();
                Ok(Pattern::Lit(Literal::Int(v, IntSuffix::None), span))
            }
            TokenKind::True => {
                let span = self.peek_span();
                self.advance();
                Ok(Pattern::Lit(Literal::Bool(true), span))
            }
            TokenKind::False => {
                let span = self.peek_span();
                self.advance();
                Ok(Pattern::Lit(Literal::Bool(false), span))
            }
            TokenKind::StringLiteral(s) => {
                let span = self.peek_span();
                let s = s.clone();
                self.advance();
                Ok(Pattern::Lit(Literal::String(s), span))
            }
            TokenKind::CharLiteral(c) => {
                let span = self.peek_span();
                self.advance();
                Ok(Pattern::Lit(Literal::Char(c), span))
            }
            _ => {
                let token = self.advance();
                Err(FrontendError::new(
                    FrontendErrorKind::UnexpectedToken {
                        expected: "pattern".to_string(),
                        found: token,
                    },
                    self.current_span(),
                ))
            }
        }
    }

    // ─── 类型 ───

    fn parse_type(&mut self) -> Result<Type, FrontendError> {
        match self.peek() {
            TokenKind::And => {
                let span = self.peek_span();
                self.advance();
                let mutable = self.eat(TokenKind::Mut);
                let inner = self.parse_type()?;
                Ok(Type::Ref(Box::new(inner), mutable, span))
            }
            TokenKind::LParen => {
                let span = self.peek_span();
                self.advance();
                if self.eat(TokenKind::RParen) {
                    return Ok(Type::Tuple(vec![], span));
                }
                let mut types = Vec::new();
                loop {
                    types.push(self.parse_type()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                Ok(Type::Tuple(types, span))
            }
            TokenKind::LBracket => {
                self.advance();
                let elem = self.parse_type()?;
                self.expect(TokenKind::Semicolon)?;
                let count = self.parse_expr(0)?;
                let span = self.expect(TokenKind::RBracket)?.span;
                Ok(Type::Array(Box::new(elem), Box::new(count), span))
            }
            TokenKind::Bang => {
                let span = self.peek_span();
                self.advance();
                Ok(Type::Never(span))
            }
            TokenKind::Underscore => {
                let span = self.peek_span();
                self.advance();
                Ok(Type::Wildcard(span))
            }
            TokenKind::Ident(_) => {
                let mut path = vec![self.expect_ident("type name")?];
                while self.eat(TokenKind::ColonColon) {
                    path.push(self.expect_ident("type name")?);
                }
                let span = self.current_span();
                let base = Type::Path(path, span);
                if self.eat(TokenKind::Lt) {
                    let mut args = Vec::new();
                    loop {
                        if self.check(TokenKind::Gt) {
                            break;
                        }
                        args.push(self.parse_type()?);
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                    let end = self.expect(TokenKind::Gt)?.span;
                    Ok(Type::Generic(Box::new(base), args, end))
                } else {
                    Ok(base)
                }
            }
            _ => {
                let token = self.advance();
                Err(FrontendError::new(
                    FrontendErrorKind::UnexpectedToken {
                        expected: "type".to_string(),
                        found: token,
                    },
                    self.current_span(),
                ))
            }
        }
    }

    // ─── stardust! 宏指令 ───

    fn parse_stardust_insts(&mut self) -> Result<Vec<StardustInst>, FrontendError> {
        let mut insts = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_eof() {
            match self.peek() {
                TokenKind::Ident(name) => {
                    let name = name.clone();
                    self.advance();
                    let inst = match name.as_str() {
                        "Push" => {
                            self.expect(TokenKind::LParen)?;
                            let expr = self.parse_expr(0)?;
                            let v = match expr {
                                Expr::Lit(Literal::Int(v, _), _) => v,
                                _ => {
                                    return Err(FrontendError::new(
                                        FrontendErrorKind::InvalidNumber(
                                            "stardust Push expects an integer literal".to_string(),
                                        ),
                                        self.current_span(),
                                    ));
                                }
                            };
                            self.expect(TokenKind::RParen)?;
                            StardustInst::Push(v)
                        }
                        "Pop" => StardustInst::Pop,
                        "Dup" => StardustInst::Dup,
                        "Swap" => StardustInst::Swap,
                        "Add" => StardustInst::Add,
                        "Sub" => StardustInst::Sub,
                        "Mul" => StardustInst::Mul,
                        "Div" => StardustInst::Div,
                        "Mod" => StardustInst::Mod,
                        "Eq" => StardustInst::Eq,
                        "Ne" => StardustInst::Ne,
                        "Lt" => StardustInst::Lt,
                        "Gt" => StardustInst::Gt,
                        "Le" => StardustInst::Le,
                        "Ge" => StardustInst::Ge,
                        "And" => StardustInst::And,
                        "Or" => StardustInst::Or,
                        "Not" => StardustInst::Not,
                        "Xor" => StardustInst::Xor,
                        "Store" => StardustInst::Store,
                        "Load" => StardustInst::Load,
                        "ShiftL" => StardustInst::ShiftL,
                        "ShiftR" => StardustInst::ShiftR,
                        "Depth" => StardustInst::Depth,
                        "Reverse" => StardustInst::Reverse,
                        "Rotate" => StardustInst::Rotate,
                        "Call" => {
                            self.expect(TokenKind::LParen)?;
                            let n = self.parse_expr_imm()?;
                            self.expect(TokenKind::Comma)?;
                            let m = self.parse_expr_imm()?;
                            self.expect(TokenKind::RParen)?;
                            StardustInst::Call { n, m }
                        }
                        "Ret" => {
                            self.expect(TokenKind::LParen)?;
                            let m = self.parse_expr_imm()?;
                            self.expect(TokenKind::RParen)?;
                            StardustInst::Ret { m }
                        }
                        _ => StardustInst::Ident(name),
                    };
                    insts.push(inst);
                }
                _ => {
                    let tok = self.advance();
                    self.errors.push(FrontendError::new(
                        FrontendErrorKind::UnexpectedToken {
                            expected: "stardust instruction".to_string(),
                            found: tok,
                        },
                        self.current_span(),
                    ));
                }
            }
        }
        Ok(insts)
    }

    fn parse_expr_imm(&mut self) -> Result<u32, FrontendError> {
        let span = self.current_span();
        let expr = self.parse_expr(0)?;
        match expr {
            Expr::Lit(Literal::Int(v, _), _) => Ok(v as u32),
            _ => Err(FrontendError::new(
                FrontendErrorKind::InvalidNumber(
                    "stardust instruction argument must be an integer literal".to_string(),
                ),
                span,
            )),
        }
    }

    // ─── match 分支 ───

    /// 解析结构体字面量字段: { x: 1, y: 2 }
    fn parse_struct_literal_fields(&mut self) -> Result<Vec<(String, Expr)>, FrontendError> {
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        if self.check(TokenKind::RBrace) {
            self.advance();
            return Ok(fields);
        }
        loop {
            let fname = self.expect_ident("field name")?;
            self.expect(TokenKind::Colon)?;
            let value = self.parse_expr(0)?;
            fields.push((fname, value));
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(fields)
    }

    fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, FrontendError> {
        let mut arms = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_eof() {
            let pattern = self.parse_pattern()?;
            let guard = if self.eat(TokenKind::If) {
                Some(Box::new(self.parse_expr(0)?))
            } else {
                None
            };
            self.expect(TokenKind::FatArrow)?;
            let body = self.parse_expr(0)?;
            let span = self.current_span();
            arms.push(MatchArm { pattern, guard, body: Box::new(body), span });

            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        Ok(arms)
    }

    // ─── 属性 ───

    fn parse_attributes(&mut self) -> Vec<Attribute> {
        let mut attrs = Vec::new();
        while self.check(TokenKind::Hash) {
            let saved = self.pos;
            self.advance();
            if self.eat(TokenKind::LBracket) {
                if let Ok(attr) = self.parse_attribute() {
                    attrs.push(attr);
                }
                let _ = self.eat(TokenKind::RBracket);
            } else {
                self.pos = saved;
                break;
            }
        }
        attrs
    }

    fn parse_attribute(&mut self) -> Result<Attribute, FrontendError> {
        let name = self.expect_ident("attribute name")?;
        let span = self.current_span();
        let mut args = Vec::new();

        if self.eat(TokenKind::LParen) {
            loop {
                if self.check(TokenKind::RParen) {
                    break;
                }
                let key = self.expect_ident("attribute arg")?;
                if self.eat(TokenKind::Eq) {
                    let value = self.expect_ident("attribute value")?;
                    args.push(AttributeArg::KeyValue(key, value));
                } else {
                    args.push(AttributeArg::Value(key));
                }
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RParen)?;
        }

        Ok(Attribute { name, args, span })
    }

    // ─── 辅助方法 ───

    /// 获取当前 token 的 kind（不推进）
    fn peek(&self) -> TokenKind {
        self.tokens.get(self.pos).map_or(TokenKind::Eof, |t| t.kind.clone())
    }

    /// 获取当前 token 的 span
    fn peek_span(&self) -> Span {
        self.tokens.get(self.pos).map_or(Span::new(0, 0, 0), |t| t.span)
    }

    fn peek_token(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&EOF_TOKEN)
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.peek() == kind
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens.get(self.pos).cloned().unwrap_or_else(|| {
            Token::new(TokenKind::Eof, Span::new(0, 0, 0))
        });
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len() || matches!(self.peek(), TokenKind::Eof)
    }

    fn current_span(&self) -> Span {
        self.peek_span()
    }

    fn pos_span(&self) -> Span {
        self.tokens.get(self.pos.saturating_sub(1)).map_or(Span::new(0, 0, 0), |t| t.span)
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token, FrontendError> {
        if self.check(kind.clone()) {
            Ok(self.advance())
        } else {
            Err(self.unexpected_token(&format!("{kind}")))
        }
    }

    fn expect_ident(&mut self, expected: &str) -> Result<String, FrontendError> {
        if let TokenKind::Ident(name) = self.peek() {
            let name = name.clone();
            self.advance();
            Ok(name)
        } else {
            let token = self.advance();
            Err(FrontendError::new(
                FrontendErrorKind::UnexpectedToken {
                    expected: expected.to_string(),
                    found: token,
                },
                self.current_span(),
            ))
        }
    }

    fn expect_string(&mut self, expected: &str) -> Result<String, FrontendError> {
        if let TokenKind::StringLiteral(s) = self.peek() {
            let s = s.clone();
            self.advance();
            Ok(s)
        } else {
            let token = self.advance();
            Err(FrontendError::new(
                FrontendErrorKind::UnexpectedToken {
                    expected: expected.to_string(),
                    found: token,
                },
                self.current_span(),
            ))
        }
    }

    fn expect_semicolon(&mut self) -> Result<(), FrontendError> {
        if self.eat(TokenKind::Semicolon) {
            Ok(())
        } else {
            Err(self.unexpected_token(";"))
        }
    }

    fn unexpected_token(&self, expected: &str) -> FrontendError {
        let token = self.peek_token();
        if matches!(token.kind, TokenKind::Eof) {
            FrontendError::new(
                FrontendErrorKind::UnexpectedEof { expected: expected.to_string() },
                token.span,
            )
        } else {
            FrontendError::new(
                FrontendErrorKind::UnexpectedToken {
                    expected: expected.to_string(),
                    found: token.clone(),
                },
                token.span,
            )
        }
    }

    fn sync_to_decl(&mut self) {
        while !self.is_eof() {
            if matches!(self.peek(),
                TokenKind::Fn | TokenKind::Struct | TokenKind::Enum |
                TokenKind::Extern | TokenKind::Use
            ) {
                break;
            }
            self.advance();
        }
    }
}

fn convert_suffix(s: &TokenIntSuffix) -> IntSuffix {
    match s {
        TokenIntSuffix::None => IntSuffix::None,
        TokenIntSuffix::I8 => IntSuffix::I8,
        TokenIntSuffix::I16 => IntSuffix::I16,
        TokenIntSuffix::I32 => IntSuffix::I32,
        TokenIntSuffix::I64 => IntSuffix::I64,
        TokenIntSuffix::Isize => IntSuffix::Isize,
        TokenIntSuffix::U8 => IntSuffix::U8,
        TokenIntSuffix::U16 => IntSuffix::U16,
        TokenIntSuffix::U32 => IntSuffix::U32,
        TokenIntSuffix::U64 => IntSuffix::U64,
        TokenIntSuffix::Usize => IntSuffix::Usize,
    }
}

static EOF_TOKEN: Token = Token {
    kind: TokenKind::Eof,
    span: Span { line: 0, column: 0, byte_pos: 0 },
};
