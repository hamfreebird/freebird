//! Freebird LSP 服务（M4-1）。
//!
//! 通过 stdio 的 JSON-RPC 提供：
//! - `textDocument/publishDiagnostics`（parse → resolve → type → borrow）
//! - `textDocument/hover`、`textDocument/definition`
//! - `textDocument/documentSymbol`、`textDocument/completion`
//!
//! 传输使用 LSP 的 `Content-Length` 帧；文档同步为 Full（`textDocumentSync: 1`）。

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use crate::diagnostics::{Diagnostic as FbDiagnostic, DiagnosticLevel, collect_diagnostics};
use crate::frontend::ast::{Block, CallArg, Decl, Expr, ExternItem, FnDecl, Pattern, Stmt};
use crate::frontend::token::Span;
use crate::io::BUILTINS;

/// LSP 服务器状态。
pub struct LspServer {
    documents: HashMap<String, String>,
    shutdown_requested: bool,
    exit_requested: bool,
}

impl LspServer {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            shutdown_requested: false,
            exit_requested: false,
        }
    }

    /// 运行 stdio 主循环。
    pub fn run_stdio(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let stdout = io::stdout();
        let mut writer = stdout.lock();

        while let Some(message) = read_message(&mut reader)? {
            let responses = self.handle(&message);
            for response in responses {
                write_message(&mut writer, &response)?;
            }
            if self.exit_requested {
                break;
            }
        }
        Ok(())
    }

    /// 处理一条 JSON-RPC 消息，返回需要写出的消息（响应或通知）。
    pub fn handle(&mut self, message: &Value) -> Vec<Value> {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();

        match method {
            "initialize" => vec![response(
                id,
                json!({
                    "capabilities": {
                        "textDocumentSync": 1,
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "renameProvider": true,
                        "documentSymbolProvider": true,
                        "completionProvider": { "triggerCharacters": [":", "."] }
                    },
                    "serverInfo": { "name": "freebird", "version": "0.1.0" }
                }),
            )],
            "initialized" | "$/cancelRequest" | "workspace/didChangeConfiguration" => Vec::new(),
            "shutdown" => {
                self.shutdown_requested = true;
                vec![response(id, Value::Null)]
            }
            "exit" => {
                self.exit_requested = true;
                Vec::new()
            }
            "textDocument/didOpen" => {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let text = params
                    .pointer("/textDocument/text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.documents.insert(uri.clone(), text);
                vec![self.publish_diagnostics(&uri)]
            }
            "textDocument/didChange" => {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(text) = params
                    .pointer("/contentChanges/0/text")
                    .and_then(Value::as_str)
                {
                    self.documents.insert(uri.clone(), text.to_string());
                }
                vec![self.publish_diagnostics(&uri)]
            }
            "textDocument/didSave" => {
                let uri = message
                    .pointer("/params/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                vec![self.publish_diagnostics(&uri)]
            }
            "textDocument/didClose" => {
                let uri = message
                    .pointer("/params/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.documents.remove(&uri);
                vec![json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": { "uri": uri, "diagnostics": [] }
                })]
            }
            "textDocument/hover" => vec![response(id, self.hover(message))],
            "textDocument/definition" => vec![response(id, self.definition(message))],
            "textDocument/references" => vec![response(id, self.references(message))],
            "textDocument/rename" => vec![response(id, self.rename(message))],
            "textDocument/documentSymbol" => vec![response(id, self.document_symbols(message))],
            "textDocument/completion" => vec![response(id, self.completion(message))],
            _ => {
                if let Some(id) = id {
                    vec![error_response(
                        id,
                        -32601,
                        &format!("method not found: {method}"),
                    )]
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn publish_diagnostics(&self, uri: &str) -> Value {
        let diagnostics = self
            .documents
            .get(uri)
            .map(|text| to_lsp_diagnostics(&collect_diagnostics(text)))
            .unwrap_or_default();
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": uri, "diagnostics": diagnostics }
        })
    }

    fn hover(&self, message: &Value) -> Value {
        let Some((text, line, character)) = self.position(message) else {
            return Value::Null;
        };
        let Some((word, range)) = word_at(&text, line, character) else {
            return Value::Null;
        };
        let Some(info) = hover_info(&text, &word) else {
            return Value::Null;
        };
        json!({
            "contents": { "kind": "markdown", "value": info },
            "range": range
        })
    }

    fn definition(&self, message: &Value) -> Value {
        let Some((text, line, character)) = self.position(message) else {
            return Value::Null;
        };
        let Some((word, _)) = word_at(&text, line, character) else {
            return Value::Null;
        };
        let uri = message
            .pointer("/params/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or("");
        match find_definition(&text, &word) {
            Some(range) => json!({ "uri": uri, "range": range }),
            None => Value::Null,
        }
    }

    fn document_symbols(&self, message: &Value) -> Value {
        let Some(text) = self.text_of(message) else {
            return Value::Null;
        };
        Value::Array(
            document_symbols(&text)
                .into_iter()
                .map(|(name, kind, line)| {
                    let range = line_range(line, 0);
                    json!({
                        "name": name,
                        "kind": kind,
                        "range": range,
                        "selectionRange": range
                    })
                })
                .collect(),
        )
    }

    fn completion(&self, message: &Value) -> Value {
        let Some(text) = self.text_of(message) else {
            return Value::Null;
        };
        let mut items: Vec<Value> = KEYWORDS
            .iter()
            .map(|keyword| json!({ "label": keyword, "kind": 14, "detail": "keyword" }))
            .collect();
        for builtin in BUILTINS {
            items.push(json!({
                "label": builtin.name,
                "kind": 3,
                "detail": builtin_signature(builtin)
            }));
        }
        for (name, kind, _) in document_symbols(&text) {
            items.push(json!({
                "label": name,
                "kind": match kind { 12 => 3, 23 => 22, 10 => 13, 2 => 9, _ => 6 },
                "detail": "document symbol"
            }));
        }
        json!({ "isIncomplete": false, "items": items })
    }

    fn references(&self, message: &Value) -> Value {
        let Some((text, line, character)) = self.position(message) else {
            return Value::Null;
        };
        let Some((word, _)) = word_at(&text, line, character) else {
            return Value::Null;
        };
        let uri = message
            .pointer("/params/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or("");
        Value::Array(
            identifier_occurrences(&text, &word)
                .into_iter()
                .map(|(line, start, end)| {
                    json!({
                        "uri": uri,
                        "range": {
                            "start": { "line": line, "character": start },
                            "end": { "line": line, "character": end }
                        }
                    })
                })
                .collect(),
        )
    }

    fn rename(&self, message: &Value) -> Value {
        let new_name = message
            .pointer("/params/newName")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !is_identifier(new_name) {
            return Value::Null;
        }
        let Some((text, line, character)) = self.position(message) else {
            return Value::Null;
        };
        let Some((word, _)) = word_at(&text, line, character) else {
            return Value::Null;
        };
        let uri = message
            .pointer("/params/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or("");
        let edits: Vec<Value> = identifier_occurrences(&text, &word)
            .into_iter()
            .map(|(line, start, end)| {
                json!({
                    "range": {
                        "start": { "line": line, "character": start },
                        "end": { "line": line, "character": end }
                    },
                    "newText": new_name
                })
            })
            .collect();
        let mut changes = serde_json::Map::new();
        changes.insert(uri.to_string(), Value::Array(edits));
        json!({ "changes": Value::Object(changes) })
    }

    fn text_of(&self, message: &Value) -> Option<String> {
        let uri = message
            .pointer("/params/textDocument/uri")
            .and_then(Value::as_str)?;
        self.documents.get(uri).cloned()
    }

    /// 返回（文档文本，0-based 行，0-based 字符列）。
    fn position(&self, message: &Value) -> Option<(String, usize, usize)> {
        let text = self.text_of(message)?;
        let line = message
            .pointer("/params/position/line")
            .and_then(Value::as_u64)? as usize;
        let character = message
            .pointer("/params/position/character")
            .and_then(Value::as_u64)? as usize;
        Some((text, line, character))
    }
}

impl Default for LspServer {
    fn default() -> Self {
        Self::new()
    }
}

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "else", "while", "loop", "for", "in", "match", "return", "ret",
    "struct", "enum", "impl", "mod", "pub", "extern", "use", "unsafe", "stardust", "true", "false",
];

fn response(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn to_lsp_diagnostics(diagnostics: &[FbDiagnostic]) -> Vec<Value> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let line = diagnostic.span.line.saturating_sub(1);
            let character = diagnostic.span.column.saturating_sub(1);
            let end_line = diagnostic
                .span
                .end_line
                .unwrap_or(diagnostic.span.line)
                .saturating_sub(1);
            let end_character = diagnostic
                .span
                .end_column
                .unwrap_or(diagnostic.span.column + 1)
                .saturating_sub(1);
            let severity = match diagnostic.level {
                DiagnosticLevel::Error => 1,
                DiagnosticLevel::Warning => 2,
                DiagnosticLevel::Note => 3,
                DiagnosticLevel::Help => 4,
            };
            let mut message = diagnostic.message.clone();
            if let Some(suggestion) = &diagnostic.suggestion {
                message.push_str("\n建议：");
                message.push_str(suggestion);
            }
            for related in &diagnostic.related {
                message.push_str(&format!(
                    "\n相关位置：{}:{} {}",
                    related.span.line, related.span.column, related.message
                ));
            }
            json!({
                "range": {
                    "start": { "line": line, "character": character },
                    "end": { "line": end_line, "character": end_character }
                },
                "severity": severity,
                "code": diagnostic.code,
                "source": "freebird",
                "message": message
            })
        })
        .collect()
}

fn line_range(line: usize, character: usize) -> Value {
    json!({
        "start": { "line": line, "character": character },
        "end": { "line": line, "character": character }
    })
}

/// 光标处的标识符及其范围（按字符计列，0-based）。
fn word_at(text: &str, line: usize, character: usize) -> Option<(String, Value)> {
    let line_text = text.lines().nth(line)?;
    let chars: Vec<char> = line_text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    // LSP 的 character 是 UTF-16 code unit；先转换为字符索引。
    let cursor = utf16_to_char_index(line_text, character).min(chars.len().saturating_sub(1));

    if !is_word_char(chars[cursor]) {
        return None;
    }
    let mut start = cursor;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = cursor + 1;
    while end < chars.len() && is_word_char(chars[end]) {
        end += 1;
    }
    let word: String = chars[start..end].iter().collect();
    Some((
        word,
        json!({
            "start": { "line": line, "character": char_to_utf16(line_text, start) },
            "end": { "line": line, "character": char_to_utf16(line_text, end) }
        }),
    ))
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn utf16_to_char_index(line: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, ch) in line.chars().enumerate() {
        if units >= utf16 {
            return index;
        }
        units += ch.len_utf16();
    }
    line.chars().count()
}

fn char_to_utf16(line: &str, char_index: usize) -> usize {
    line.chars().take(char_index).map(char::len_utf16).sum()
}

fn byte_to_utf16(line: &str, byte_index: usize) -> usize {
    line[..byte_index].encode_utf16().count()
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(is_word_char)
}

/// 查找完整标识符出现位置；返回（0-based 行，UTF-16 起止列）。
/// 语义优先的标识符出现位置：解析成功时只返回 AST 中的变量引用/绑定，
/// 解析失败时回退到纯文本匹配。
fn identifier_occurrences(text: &str, word: &str) -> Vec<(usize, usize, usize)> {
    match semantic_occurrences(text, word) {
        Some(occurrences) if !occurrences.is_empty() => occurrences,
        _ => word_occurrences(text, word),
    }
}

/// 基于 AST 收集 `word` 的引用与绑定位置（排除注释/字符串中的同名文本）。
fn semantic_occurrences(text: &str, word: &str) -> Option<Vec<(usize, usize, usize)>> {
    let (program, errors) = crate::frontend::parse(text);
    if !errors.is_empty() {
        return None;
    }
    let mut spans: Vec<Span> = Vec::new();
    for decl in &program.decls {
        match decl {
            Decl::Fn(f) => collect_fn_occurrences(f, word, &mut spans),
            Decl::Impl(i) => {
                for f in &i.fns {
                    collect_fn_occurrences(f, word, &mut spans);
                }
            }
            Decl::Trait(t) => {
                for f in &t.methods {
                    collect_fn_occurrences(f, word, &mut spans);
                }
            }
            Decl::Extern(e) => {
                for item in &e.items {
                    let ExternItem::Fn(f) = item;
                    collect_fn_occurrences(f, word, &mut spans);
                }
            }
            Decl::Const(c) => collect_expr_occurrences(&c.value, word, &mut spans),
            _ => {}
        }
    }
    let mut occurrences: Vec<(usize, usize, usize)> = spans
        .into_iter()
        .map(|span| {
            let line = span.line.saturating_sub(1);
            let start = span.column.saturating_sub(1);
            let end = start + word.chars().map(|c| c.len_utf16()).sum::<usize>();
            (line, start, end)
        })
        .collect();
    occurrences.sort_unstable();
    occurrences.dedup();
    Some(occurrences)
}

fn collect_fn_occurrences(f: &FnDecl, word: &str, out: &mut Vec<Span>) {
    // 函数名本身不是 `Expr::Var`；`fn` 关键字后紧跟一个空格和函数名。
    if f.name == word {
        out.push(Span {
            line: f.span.line,
            column: f.span.column + 3,
            byte_pos: f.span.byte_pos + 3,
            file_id: f.span.file_id,
        });
    }
    for param in &f.params {
        if param.name == word {
            out.push(param.span);
        }
    }
    if let Some(body) = &f.body {
        collect_block_occurrences(body, word, out);
    }
}

fn collect_block_occurrences(block: &Block, word: &str, out: &mut Vec<Span>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { pattern, init, .. } => {
                collect_pattern_occurrences(pattern, word, out);
                if let Some(init) = init {
                    collect_expr_occurrences(init, word, out);
                }
            }
            Stmt::Expr(expr, _) => collect_expr_occurrences(expr, word, out),
            Stmt::Assign { target, value, .. } => {
                collect_expr_occurrences(target, word, out);
                collect_expr_occurrences(value, word, out);
            }
        }
    }
    if let Some(tail) = &block.tail {
        collect_expr_occurrences(tail, word, out);
    }
}

fn collect_pattern_occurrences(pattern: &Pattern, word: &str, out: &mut Vec<Span>) {
    match pattern {
        Pattern::Var(name, span) if name == word => out.push(*span),
        Pattern::Tuple(patterns, _, _) => {
            for p in patterns {
                collect_pattern_occurrences(p, word, out);
            }
        }
        Pattern::Struct { fields, .. } => {
            for p in fields.iter().filter_map(|(_, p)| p.as_ref()) {
                collect_pattern_occurrences(p, word, out);
            }
        }
        Pattern::Enum {
            inner: Some(inner), ..
        } => collect_pattern_occurrences(inner, word, out),
        Pattern::Enum { inner: None, .. } => {}
        Pattern::Ref(inner, _, _) => collect_pattern_occurrences(inner, word, out),
        Pattern::Or(patterns, _) => {
            for p in patterns {
                collect_pattern_occurrences(p, word, out);
            }
        }
        _ => {}
    }
}

fn collect_expr_occurrences(expr: &Expr, word: &str, out: &mut Vec<Span>) {
    match expr {
        Expr::Var(name, span) if name == word => out.push(*span),
        Expr::Closure { params, body, .. } => {
            for param in params {
                if param.name == word {
                    out.push(param.span);
                }
            }
            collect_expr_occurrences(body, word, out);
        }
        Expr::Deref(e, _)
        | Expr::Ref(e, _, _)
        | Expr::Unary(_, e, _)
        | Expr::FieldAccess(e, _, _)
        | Expr::As(e, _, _)
        | Expr::TypeArgs(e, _, _) => collect_expr_occurrences(e, word, out),
        Expr::Binary(l, _, r, _) | Expr::Index(l, r, _) | Expr::Repeat(l, r, _) => {
            collect_expr_occurrences(l, word, out);
            collect_expr_occurrences(r, word, out);
        }
        Expr::Call { func, args, .. } => {
            collect_expr_occurrences(func, word, out);
            for arg in args {
                match arg {
                    CallArg::Pos(e, _) | CallArg::Named(_, e, _) => {
                        collect_expr_occurrences(e, word, out);
                    }
                }
            }
        }
        Expr::Tuple(exprs, _) | Expr::Array(exprs, _) => {
            for e in exprs {
                collect_expr_occurrences(e, word, out);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, e) in fields {
                collect_expr_occurrences(e, word, out);
            }
        }
        Expr::EnumLit { data: Some(e), .. } => {
            collect_expr_occurrences(e, word, out);
        }
        Expr::EnumLit { data: None, .. } => {}
        Expr::If(cond, then, else_, _) => {
            collect_expr_occurrences(cond, word, out);
            collect_block_occurrences(then, word, out);
            if let Some(else_) = else_ {
                collect_block_occurrences(else_, word, out);
            }
        }
        Expr::Match(scrutinee, arms, _) => {
            collect_expr_occurrences(scrutinee, word, out);
            for arm in arms {
                collect_pattern_occurrences(&arm.pattern, word, out);
                if let Some(guard) = &arm.guard {
                    collect_expr_occurrences(guard, word, out);
                }
                collect_expr_occurrences(&arm.body, word, out);
            }
        }
        Expr::Loop(block, _) | Expr::Unsafe(block, _) => {
            collect_block_occurrences(block, word, out);
        }
        Expr::While(cond, block, _) => {
            collect_expr_occurrences(cond, word, out);
            collect_block_occurrences(block, word, out);
        }
        Expr::For {
            pattern,
            iterable,
            body,
            ..
        } => {
            collect_pattern_occurrences(pattern, word, out);
            collect_expr_occurrences(iterable, word, out);
            collect_block_occurrences(body, word, out);
        }
        Expr::Block(block, _) => collect_block_occurrences(block, word, out),
        Expr::Labelled(_, e, _) => collect_expr_occurrences(e, word, out),
        Expr::Break(Some(e), _) | Expr::BreakLabel(_, Some(e), _) => {
            collect_expr_occurrences(e, word, out);
        }
        Expr::Return(exprs, _) => {
            for e in exprs {
                collect_expr_occurrences(e, word, out);
            }
        }
        _ => {}
    }
}

fn word_occurrences(text: &str, word: &str) -> Vec<(usize, usize, usize)> {
    let mut occurrences = Vec::new();
    if word.is_empty() {
        return occurrences;
    }
    for (line_index, line) in text.lines().enumerate() {
        let mut search = 0;
        while let Some(relative) = line[search..].find(word) {
            let start = search + relative;
            let end = start + word.len();
            let before_ok = line[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !is_word_char(c));
            let after_ok = line[end..].chars().next().is_none_or(|c| !is_word_char(c));
            if before_ok && after_ok {
                occurrences.push((
                    line_index,
                    byte_to_utf16(line, start),
                    byte_to_utf16(line, end),
                ));
            }
            search = end.max(start + 1);
        }
    }
    occurrences
}

fn hover_info(text: &str, word: &str) -> Option<String> {
    if let Some(builtin) = BUILTINS.iter().find(|b| b.name == word) {
        return Some(format!("```freebird\n{}\n```", builtin_signature(builtin)));
    }
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("fn ")
            && rest.starts_with(&format!("{word}("))
        {
            return Some(format!("```freebird\n{trimmed}\n```"));
        }
        if let Some(rest) = trimmed.strip_prefix("pub fn ")
            && rest.starts_with(&format!("{word}("))
        {
            return Some(format!("```freebird\n{trimmed}\n```"));
        }
        if let Some(rest) = trimmed.strip_prefix("let ")
            && let Some(after) = rest.strip_prefix(&format!("{word}:"))
        {
            return Some(format!(
                "```freebird\nlet {word}:{}\n```",
                after.trim_end_matches(';')
            ));
        }
    }
    None
}

fn builtin_signature(builtin: &crate::io::BuiltinDef) -> String {
    let params = builtin.params.join(", ");
    if builtin.returns.is_empty() {
        format!("{}({params})", builtin.name)
    } else {
        format!(
            "{}({params}) -> {}",
            builtin.name,
            builtin.returns.join(", ")
        )
    }
}

/// 查找声明位置，返回 LSP Range。
fn find_definition(text: &str, word: &str) -> Option<Value> {
    for (line_index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let trimmed = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
        for keyword in ["fn ", "struct ", "enum ", "impl ", "let "] {
            if let Some(rest) = trimmed.strip_prefix(keyword)
                && let Some(rest) = rest.strip_prefix(word)
                && rest
                    .chars()
                    .next()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
            {
                let start = line.find(word).unwrap_or(0);
                return Some(json!({
                    "start": { "line": line_index, "character": byte_to_utf16(line, start) },
                    "end": { "line": line_index, "character": byte_to_utf16(line, start + word.len()) }
                }));
            }
        }
    }
    None
}

/// 文档符号：（名字，SymbolKind，0-based 行）。
fn document_symbols(text: &str) -> Vec<(String, u64, usize)> {
    let mut symbols = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let trimmed = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
        let candidates: [(&str, u64); 5] = [
            ("fn ", 12),
            ("struct ", 23),
            ("enum ", 10),
            ("impl ", 5),
            ("mod ", 2),
        ];
        for (keyword, kind) in candidates {
            let Some(rest) = trimmed.strip_prefix(keyword) else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            if !name.is_empty() {
                symbols.push((name, kind, line_index));
            }
            break;
        }
    }
    symbols
}

/// 读取一条 LSP 消息（Content-Length 帧）。
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }

    let Some(length) = content_length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Content-Length header",
        ));
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let value = serde_json::from_slice(&body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(value))
}

/// 写出一条 LSP 消息。
pub fn write_message<W: Write>(writer: &mut W, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn did_open(text: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": "file:///test.fb", "languageId": "freebird", "version": 1, "text": text }
            }
        })
    }

    #[test]
    fn initialize_advertises_capabilities() {
        let mut server = LspServer::new();
        let responses = server.handle(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}
        }));
        assert_eq!(responses.len(), 1);
        assert_eq!(
            responses[0]["result"]["capabilities"]["hoverProvider"],
            true
        );
        assert_eq!(
            responses[0]["result"]["capabilities"]["textDocumentSync"],
            1
        );
    }

    #[test]
    fn did_open_publishes_diagnostics_with_codes() {
        let mut server = LspServer::new();
        let responses = server.handle(&did_open("fn main() { let x: i32 = true; }\n"));
        let params = &responses[0]["params"];
        assert_eq!(params["uri"], "file:///test.fb");
        let diagnostics = params["diagnostics"].as_array().unwrap();
        assert!(!diagnostics.is_empty());
        assert_eq!(diagnostics[0]["code"], "T001");
        assert_eq!(diagnostics[0]["source"], "freebird");
    }

    #[test]
    fn did_change_updates_diagnostics() {
        let mut server = LspServer::new();
        server.handle(&did_open("fn main() { let x: i32 = true; }\n"));
        let responses = server.handle(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": "file:///test.fb", "version": 2 },
                "contentChanges": [{ "text": "fn main() { let x: i32 = 1; }\n" }]
            }
        }));
        assert_eq!(
            responses[0]["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn hover_and_definition_and_symbols() {
        let mut server = LspServer::new();
        let text = "fn add(a: i64, b: i64) -> i64 { a + b }\nfn main() { print(add(1, 2)); }\n";
        server.handle(&did_open(text));

        let hover = server.handle(&json!({
            "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
            "params": { "textDocument": { "uri": "file:///test.fb" }, "position": { "line": 1, "character": 19 } }
        }));
        assert!(
            hover[0]["result"]["contents"]["value"]
                .as_str()
                .unwrap()
                .contains("fn add")
        );

        let definition = server.handle(&json!({
            "jsonrpc": "2.0", "id": 3, "method": "textDocument/definition",
            "params": { "textDocument": { "uri": "file:///test.fb" }, "position": { "line": 1, "character": 19 } }
        }));
        assert_eq!(definition[0]["result"]["range"]["start"]["line"], 0);

        let symbols = server.handle(&json!({
            "jsonrpc": "2.0", "id": 4, "method": "textDocument/documentSymbol",
            "params": { "textDocument": { "uri": "file:///test.fb" } }
        }));
        let names: Vec<&str> = symbols[0]["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|symbol| symbol["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["add", "main"]);
    }

    #[test]
    fn completion_contains_builtins_and_keywords() {
        let mut server = LspServer::new();
        server.handle(&did_open("fn main() {}\n"));
        let response = server.handle(&json!({
            "jsonrpc": "2.0", "id": 5, "method": "textDocument/completion",
            "params": { "textDocument": { "uri": "file:///test.fb" }, "position": { "line": 0, "character": 0 } }
        }));
        let labels: Vec<&str> = response[0]["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"fn"));
        assert!(labels.contains(&"print"));
        assert!(labels.contains(&"main"));
    }

    #[test]
    fn content_length_round_trip() {
        let message = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let mut buffer = Vec::new();
        write_message(&mut buffer, &message).unwrap();
        let mut reader = std::io::Cursor::new(buffer);
        let parsed = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(parsed, message);
    }

    #[test]
    fn semantic_occurrences_ignore_comments_and_strings() {
        let source = "fn main() {\n    let x = 1;\n    // x in a comment\n    print_str(\"x\");\n    print(x);\n}\n";
        let occurrences = identifier_occurrences(source, "x");
        assert_eq!(occurrences.len(), 2, "occurrences: {occurrences:?}");
        assert!(
            occurrences
                .iter()
                .all(|(line, _, _)| *line == 1 || *line == 4),
            "unexpected lines: {occurrences:?}"
        );
    }

    #[test]
    fn references_and_rename_work() {
        let mut server = LspServer::new();
        let text = "fn add(a: i64, b: i64) -> i64 { a + b }\nfn main() { print(add(1, 2)); print(add(3, 4)); }\n";
        server.handle(&did_open(text));
        let references = server.handle(&json!({
            "jsonrpc": "2.0", "id": 10, "method": "textDocument/references",
            "params": {
                "textDocument": { "uri": "file:///test.fb" },
                "position": { "line": 1, "character": 19 },
                "context": { "includeDeclaration": true }
            }
        }));
        let locations = references[0]["result"].as_array().unwrap();
        assert_eq!(locations.len(), 3, "expected definition + 2 calls");
        assert_eq!(locations[0]["range"]["start"]["line"], 0);

        let rename = server.handle(&json!({
            "jsonrpc": "2.0", "id": 11, "method": "textDocument/rename",
            "params": {
                "textDocument": { "uri": "file:///test.fb" },
                "position": { "line": 1, "character": 19 },
                "newName": "sum"
            }
        }));
        let edits = rename[0]["result"]["changes"]["file:///test.fb"]
            .as_array()
            .unwrap();
        assert_eq!(edits.len(), 3);
        assert!(edits.iter().all(|edit| edit["newText"] == "sum"));
    }

    #[test]
    fn utf16_positions_handle_non_bmp_characters() {
        let mut server = LspServer::new();
        let line = "    let s: String = \"😀\"; print(s);";
        let text = format!("fn main() {{\n{line}\n}}\n");
        server.handle(&did_open(&text));
        let byte = line.rfind('s').unwrap();
        let character = line[..byte].encode_utf16().count();
        let hover = server.handle(&json!({
            "jsonrpc": "2.0", "id": 12, "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///test.fb" },
                "position": { "line": 1, "character": character }
            }
        }));
        let value = hover[0]["result"]["contents"]["value"].as_str().unwrap();
        assert!(value.contains("let s:"), "unexpected hover: {value}");
    }
}
