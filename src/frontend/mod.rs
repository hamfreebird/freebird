pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod token;

#[cfg(test)]
mod parser_tests;

use lexer::Lexer;
use parser::Parser;

/// 解析 Freebird 源码为 AST
pub fn parse(source: &str) -> (ast::Program, Vec<error::FrontendError>) {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize();
    let lex_errors = lexer.errors().to_vec();

    let mut parser = Parser::new(tokens);
    let program = parser.parse_program();
    let mut parse_errors = parser.errors().to_vec();

    let mut all_errors = lex_errors;
    all_errors.append(&mut parse_errors);

    (program, all_errors)
}
