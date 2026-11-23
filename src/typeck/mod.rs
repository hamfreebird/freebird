pub mod checker;
pub mod error;
pub mod type_stack;
pub mod type_table;

pub use checker::{
    check_program, check_program_with_tags, check_program_with_warnings, MethodResolution,
    MethodTagMap, MethodVariantTags, TypeCheckError, TypeCheckWarning, TypeChecker,
};
pub use type_stack::TypeStack;
pub use type_table::TypeTable;
