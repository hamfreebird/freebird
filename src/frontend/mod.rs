pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod token;

#[cfg(test)]
mod parser_tests;

use std::path::{Path, PathBuf};

use lexer::Lexer;
use parser::Parser;

/// 多文件项目的文件映射：`Span.file_id` → 文件路径。
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<PathBuf>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, path: impl Into<PathBuf>) -> u32 {
        let id = self.files.len() as u32;
        self.files.push(path.into());
        id
    }

    pub fn path(&self, id: u32) -> Option<&Path> {
        self.files.get(id as usize).map(PathBuf::as_path)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// 解析 Freebird 源码为 AST
pub fn parse(source: &str) -> (ast::Program, Vec<error::FrontendError>) {
    parse_with_file(source, 0)
}

/// 解析单个文件，并把 `file_id` 写入所有 Span。
pub fn parse_with_file(source: &str, file_id: u32) -> (ast::Program, Vec<error::FrontendError>) {
    let mut lexer = Lexer::with_file(source, file_id);
    let tokens = lexer.tokenize();
    let lex_errors = lexer.errors().to_vec();

    let mut parser = Parser::new(tokens);
    let program = parser.parse_program();
    let mut parse_errors = parser.errors().to_vec();

    let mut all_errors = lex_errors;
    all_errors.append(&mut parse_errors);

    (program, all_errors)
}
