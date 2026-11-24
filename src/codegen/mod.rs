pub mod ffi;
pub mod llvm;
pub mod optimize;
pub mod stardust;

pub use llvm::{
    LlvmArtifact, LlvmEmit, LlvmOptions, compile_llvm, compile_llvm_compiled, compile_llvm_ir,
};
pub use optimize::{constant_leaf_functions, inline_constant_calls, optimize_mir};
pub use stardust::StardustGen;
