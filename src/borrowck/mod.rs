pub mod checker;
pub mod drop_gen;

pub use checker::BorrowChecker;
pub use drop_gen::DropGen;

/// 对整个 HIR 程序执行借用/移动检查。
pub fn check_borrow(program: &crate::hir::types::HirProgram) -> Vec<checker::BorrowError> {
    let mut checker = BorrowChecker::new();
    checker.check_program(program);
    checker.errors
}
