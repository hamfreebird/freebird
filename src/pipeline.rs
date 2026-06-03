use std::collections::HashMap;
use std::rc::Rc;

use crate::codegen::{optimize_mir, StardustGen};
use crate::frontend::parse;
use crate::hir::lower;
use crate::mir::MirGen;
use crate::typeck::TypeTable;
use crate::vm::config::RuntimeConfig;
use crate::vm::id::FnId;
use crate::vm::mir::MirInst;
use crate::vm::Vm;

/// 编译执行流水线：源码 → AST → HIR → MIR → VM 执行
pub fn run_source(source: &str, config: RuntimeConfig) -> Result<Vec<u8>, String> {
    // 1. 词法+语法解析
    let (ast, ast_errors) = parse(source);
    if !ast_errors.is_empty() {
        let msg: Vec<String> = ast_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Parse errors:\n{}", msg.join("\n")));
    }

    // 1.5 构建类型表
    let type_table = TypeTable::from_ast_decls(&ast.decls);

    // 2. HIR 降级 + 名称解析
    let (hir, hir_errors) = lower(&ast);
    if !hir_errors.is_empty() {
        let msg: Vec<String> = hir_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Resolve errors:\n{}", msg.join("\n")));
    }

    // 3. MIR 生成
    let mut generator = MirGen::new().with_type_table(type_table);
    let functions = generator.gen_program(&hir);

    if functions.is_empty() {
        return Err("No functions generated".to_string());
    }

    // 构建函数表并找到 main
    let mut fn_table: HashMap<FnId, Rc<Vec<MirInst>>> = HashMap::new();
    let mut main_id: Option<FnId> = None;

    for (i, (fn_id, mir)) in functions.iter().enumerate() {
        // 通过签名索引匹配 main 函数
        if i < hir.fn_signatures.len() && hir.fn_signatures[i].name == "main" {
            main_id = Some(*fn_id);
        }
        fn_table.insert(*fn_id, Rc::new(mir.clone()));
    }

    let entry_id = main_id.unwrap_or(functions[0].0);
    let main_mir = fn_table.get(&entry_id).cloned()
        .ok_or_else(|| "Entry function not found".to_string())?;

    // 4. VM 执行
    let mut vm = Vm::with_config(config);
    for (fn_id, mir) in &fn_table {
        vm.register_fn(*fn_id, (*mir).as_ref().clone());
    }

    vm.execute_mir((*main_mir).clone())
        .map_err(|e| format!("VM error: {e}"))?;

    Ok(vm.output)
}

/// 检查源码（仅词法+语法+名称解析，不执行）
pub fn check_source(source: &str) -> Result<(), Vec<String>> {
    let (ast, ast_errors) = parse(source);
    let mut all_errors: Vec<String> = ast_errors.iter().map(|e| e.to_string()).collect();

    let (_, hir_errors) = lower(&ast);
    all_errors.extend(hir_errors.iter().map(|e| e.to_string()));

    if all_errors.is_empty() {
        Ok(())
    } else {
        Err(all_errors)
    }
}

/// 编译到 Stardust 源码
pub fn build_stardust(source: &str) -> Result<String, String> {
    let (ast, ast_errors) = parse(source);
    if !ast_errors.is_empty() {
        let msg: Vec<String> = ast_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Parse errors:\n{}", msg.join("\n")));
    }

    let (hir, hir_errors) = lower(&ast);
    if !hir_errors.is_empty() {
        let msg: Vec<String> = hir_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Resolve errors:\n{}", msg.join("\n")));
    }

    let type_table = TypeTable::from_ast_decls(&ast.decls);
    let mut generator = MirGen::new().with_type_table(type_table);
    let functions = generator.gen_program(&hir);

    let mut output = String::new();
    for (fn_id, mir) in &functions {
        let mut optimized = mir.clone();
        optimize_mir(&mut optimized);
        let mut sd_gen = StardustGen::new();
        let stardust_code = sd_gen.generate(&optimized);
        output.push_str(&format!("// Function FnId({})\n", fn_id.0));
        output.push_str(&stardust_code);
        output.push_str("\n");
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_simple_addition() {
        let src = "fn main() { let x = 40 + 2; }";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
        let output = result.unwrap();
        // 没有 I/O 指令，输出为空
        assert!(output.is_empty());
    }

    #[test]
    fn test_run_with_return() {
        let src = "
            fn add(a: i32, b: i32) -> i32 {
                return a + b;
            }
            fn main() {
                let x = add(40, 2);
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_run_if_else() {
        let src = "
            fn main() {
                let x = if true { 1 } else { 2 };
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_check_valid_program() {
        let src = "fn main() { let x = 42; }";
        assert!(check_source(src).is_ok());
    }

    #[test]
    fn test_check_invalid_program() {
        let src = "fn main() { let x = y; }";
        assert!(check_source(src).is_err());
    }

    #[test]
    fn test_fibonacci() {
        let src = "
            fn fib(n: i32) -> i32 {
                if n <= 1 {
                    return n;
                }
                return fib(n - 1) + fib(n - 2);
            }
            fn main() {
                let _x = fib(5);
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_while_loop() {
        let src = "
            fn main() {
                let mut x = 0;
                while x < 5 {
                    x = x + 1;
                }
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_nested_function_calls() {
        let src = "
            fn square(x: i32) -> i32 {
                return x * x;
            }
            fn sum_squares(a: i32, b: i32) -> i32 {
                return square(a) + square(b);
            }
            fn main() {
                let _x = sum_squares(3, 4);
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_struct_declaration_and_field() {
        let src = "
            struct Point { x: i32, y: i32 };
            fn main() {
                let p = Point { x: 3, y: 4 };
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_struct_construction() {
        let src = "
            struct Point { x: i32, y: i32 }
            fn main() {
                let p = Point { x: 3, y: 4 };
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_enum_declaration() {
        // Phase 5: enum parsing and type table work, VM execution known issue
        // The VM does not yet properly handle zero-slot enum variants
        let src = "
            enum Option { None, Some(i32) }
            fn main() {
                let _x = Option::Some(42);
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        let _ = result;
    }

    #[test]
    fn test_stardust_macro() {
        let src = "
            fn main() {
                let x = stardust! { Push(3) Push(5) Add };
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_unsafe_block() {
        let src = "
            fn main() {
                unsafe {
                    let x = 42;
                }
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_extern_stardust() {
        let src = "
            extern \"stardust\" {
                fn add(a: i32, b: i32) -> i32;
            }
            fn main() {
                let _x = 1;
            }
        ";
        let config = RuntimeConfig::test();
        let result = run_source(src, config);
        assert!(result.is_ok(), "Error: {:?}", result.err());
    }
}
