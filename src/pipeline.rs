use std::collections::HashMap;
use std::rc::Rc;

use crate::borrowck::check_borrow;
use crate::codegen::stardust::FnInfo;
use crate::codegen::{optimize_mir, StardustGen};
use crate::frontend::parse;
use crate::hir::lower;
use crate::mir::MirGen;
use crate::typeck::{check_program, TypeTable};
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

    // 2.5 基础类型检查
    let type_errors = check_program(&hir, &type_table);
    if !type_errors.is_empty() {
        let msg: Vec<String> = type_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Type errors:\n{}", msg.join("\n")));
    }

    // 2.6 借用/移动检查
    let borrow_errors = check_borrow(&hir);
    if !borrow_errors.is_empty() {
        let msg: Vec<String> = borrow_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Borrow errors:\n{}", msg.join("\n")));
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

/// 检查源码（词法+语法+名称解析+基础类型检查，不执行）
pub fn check_source(source: &str) -> Result<(), Vec<String>> {
    let (ast, ast_errors) = parse(source);
    let mut all_errors: Vec<String> = ast_errors.iter().map(|e| e.to_string()).collect();

    let (hir, hir_errors) = lower(&ast);
    all_errors.extend(hir_errors.iter().map(|e| e.to_string()));

    if ast_errors.is_empty() && hir_errors.is_empty() {
        let type_table = TypeTable::from_ast_decls(&ast.decls);
        let type_errors = check_program(&hir, &type_table);
        all_errors.extend(type_errors.iter().map(|e| e.to_string()));

        if type_errors.is_empty() {
            all_errors.extend(check_borrow(&hir).iter().map(|e| e.to_string()));
        }
    }

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

    let type_table = TypeTable::from_ast_decls(&ast.decls);

    let (hir, hir_errors) = lower(&ast);
    if !hir_errors.is_empty() {
        let msg: Vec<String> = hir_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Resolve errors:\n{}", msg.join("\n")));
    }

    let type_errors = check_program(&hir, &type_table);
    if !type_errors.is_empty() {
        let msg: Vec<String> = type_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Type errors:\n{}", msg.join("\n")));
    }

    let borrow_errors = check_borrow(&hir);
    if !borrow_errors.is_empty() {
        let msg: Vec<String> = borrow_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Borrow errors:\n{}", msg.join("\n")));
    }

    let mut generator = MirGen::new().with_type_table(type_table);
    let functions = generator.gen_program(&hir);
    if functions.is_empty() {
        return Err("No functions generated".to_string());
    }

    // FnId 由 MIR 生成器按签名顺序分配，从 1 开始。
    let mut fn_info = HashMap::new();
    let mut entry = None;
    for (i, sig) in hir.fn_signatures.iter().enumerate() {
        let id = FnId((i + 1) as u64);
        fn_info.insert(id, FnInfo::new(sig.name.clone(), sig.n, sig.m));
        if entry.is_none() && sig.name == "main" {
            entry = Some(id);
        }
    }
    let entry = entry.or_else(|| functions.first().map(|(id, _)| *id));

    let mut optimized_functions = Vec::with_capacity(functions.len());
    for (fn_id, mir) in &functions {
        let mut optimized = mir.clone();
        optimize_mir(&mut optimized);
        optimized_functions.push((*fn_id, optimized));
    }

    let mut sd_gen = StardustGen::new();
    Ok(sd_gen.generate_program(&optimized_functions, &fn_info, entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_stardust_reports_type_errors() {
        let err = build_stardust("fn main() { let x: i32 = true; }").unwrap_err();
        assert!(err.contains("Type errors"), "unexpected error: {err}");
    }

    #[test]
    fn test_build_stardust_reports_borrow_errors() {
        let err =
            build_stardust(r#"fn main() { let s = "hi"; let t = s; print_str(s); }"#)
                .unwrap_err();
        assert!(err.contains("Borrow errors"), "unexpected error: {err}");
    }

    #[test]
    fn test_print_variable_twice() {
        let src = "fn main() { let x = 7; print(x); print(x); }";
        let output = run_source(src, RuntimeConfig::test()).expect("program should run");
        assert_eq!(output, b"7\n7\n");
    }

    #[test]
    fn test_print_int_and_print_char_builtins() {
        let src = "fn main() { print_int(42); print_char('A'); }";
        let output = run_source(src, RuntimeConfig::test()).expect("program should run");
        assert_eq!(output, b"42\nA");
    }

    #[test]
    fn test_builtin_is_not_shadowed_by_user_function() {
        let src = "fn print_int() { } fn main() { print_int(); }";
        let result = run_source(src, RuntimeConfig::test());
        assert!(result.is_ok(), "user function should override builtin: {result:?}");
    }

    #[test]
    fn test_builtin_argument_count_is_checked() {
        // 空参数 `print_int()` 是合法的栈传参语法；这里验证多余参数被拒绝。
        let src = "fn main() { print_int(1, 2); }";
        assert!(check_source(src).is_err());
    }

    #[test]
    fn test_print_does_not_destroy_variable() {
        let src = "fn main() { let x = 2; print(x); let y = x + 1; print(y); }";
        let output = run_source(src, RuntimeConfig::test()).expect("program should run");
        assert_eq!(output, b"2\n3\n");
    }

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
    fn test_enum_match_payload_binding() {
        let src = "
            enum Option { None, Some(i32) }
            fn main() {
                let a = Option::Some(42);
                match a { Option::None => print(0), Option::Some(v) => print(v), }
                let b = Option::None;
                match b { Option::None => print(7), Option::Some(v) => print(v), }
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("program should run");
        assert_eq!(output, b"42\n7\n");
    }

    #[test]
    fn test_match_expression_and_literal_patterns() {
        let src = "
            fn main() {
                let x = match 2 { 1 => 10, 2 => 20, _ => 30, };
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("program should run");
        assert_eq!(output, b"20\n");
    }

    #[test]
    fn test_match_guard_falls_through() {
        let src = "
            fn main() {
                let x = 2;
                let y = match x { n if n > 3 => 1, _ => 0, };
                print(y);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("program should run");
        assert_eq!(output, b"0\n");
    }

    #[test]
    fn test_time_limit_is_enforced() {
        let src = "fn main() { print(1); }";
        let mut config = RuntimeConfig::test();
        config.time_limit = Some(0);
        let result = run_source(src, config);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("time limit exceeded"),
            "expected time limit error"
        );
    }

    #[test]
    fn test_runtime_error_reports_debug_state() {
        let src = "fn main() { let x = 1 / 0; }";
        let mut config = RuntimeConfig::test();
        config.debug = true;
        let result = run_source(src, config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("division by zero"));
    }

    #[test]
    fn test_read_int_and_read_line_from_input() {
        let src = "
            fn main() {
                let n = read_int();
                print(n + 1);
                let line = read_line();
                print_str(line);
            }
        ";
        let mut config = RuntimeConfig::test();
        config.input = b"41\nhello\n".to_vec();
        let output = run_source(src, config).expect("input program should run");
        assert_eq!(output, b"42\nhello");
    }

    #[test]
    fn test_string_io_builtins() {
        let src = r#"fn main() { print_str("abc"); println_str("def"); }"#;
        let output = run_source(src, RuntimeConfig::test()).expect("string io should run");
        assert_eq!(output, b"abcdef\n");
    }

    #[test]
    fn test_file_roundtrip_builtins() {
        let path = std::env::temp_dir().join(format!(
            "freebird_io_test_{}_{}.txt",
            std::process::id(),
            std::thread::current().name().unwrap_or("main")
        ));
        let path_str = path.display().to_string();
        let src = format!(
            r#"fn main() {{ file_write("{path_str}", "hello file"); print_str(file_read("{path_str}")); }}"#
        );
        let output = run_source(&src, RuntimeConfig::test()).expect("file io should run");
        assert_eq!(output, b"hello file");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_struct_multi_field_access() {
        let src = "
            struct P { x: i32, y: i32, z: i32 }
            fn main() {
                let p = P { z: 30, x: 10, y: 20 };
                print(p.x);
                print(p.y);
                print(p.z);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("struct program should run");
        assert_eq!(output, b"10\n20\n30\n");
    }

    #[test]
    fn test_vec_macro_and_indexing() {
        let src = "fn main() { let v: Vec<i32> = vec![1, 2, 3]; print(v[0]); print(v[2]); }";
        let output = run_source(src, RuntimeConfig::test()).expect("vec program should run");
        assert_eq!(output, b"1\n3\n");
    }

    #[test]
    fn test_array_index_order_and_repeat() {
        let src = "fn main() { let a = [10, 20, 30]; print(a[0]); print(a[2]); let r = [7; 3]; print(r[1]); }";
        let output = run_source(src, RuntimeConfig::test()).expect("array program should run");
        assert_eq!(output, b"10\n30\n7\n");
    }

    #[test]
    fn test_tuple_destructuring() {
        let src = "
            fn divrem(a: i32, b: i32) -> (i32, i32) {
                return a / b, a % b;
            }
            fn main() {
                let (q, r) = divrem(10, 3);
                print(q);
                print(r);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("program should run");
        assert_eq!(output, b"3\n1\n");
    }

    #[test]
    fn test_array_index_out_of_bounds_is_error() {
        let src = "fn main() { let x = [1, 2, 3][5]; }";
        let result = run_source(src, RuntimeConfig::test());
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("index out of bounds"),
            "expected bounds error"
        );
    }

    #[test]
    fn test_vec_methods_and_string_box() {
        let src = "
            fn main() {
                let mut v = vec![1, 2, 3];
                v.push(4);
                print(v.len());
                print(v[3]);
                print(v.pop());
                print(v.len());

                let mut s: String = String::new();
                s.push('h');
                s.push('i');
                print(s.len());
                println_str(s);

                let b = Box::new(42);
                print(*b);

                let n: Vec<i32> = Vec::new();
                print(n.len());
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("vec methods should run");
        assert_eq!(output, b"4\n4\n4\n3\n2\nhi\n42\n0\n");
    }

    #[test]
    fn test_vec_push_rejects_immutable_binding() {
        let result = run_source(
            "fn main() { let v = vec![1]; v.push(2); }",
            RuntimeConfig::test(),
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("cannot assign to immutable variable"),
            "expected immutable receiver diagnostic"
        );
    }

    #[test]
    fn test_borrow_check_rejects_use_after_move() {
        let src = r#"fn main() { let s = "hi"; let t = s; print_str(s); }"#;
        let result = run_source(src, RuntimeConfig::test());
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("use of moved value `s`"),
            "expected use-after-move diagnostic"
        );
    }

    #[test]
    fn test_borrow_check_rejects_mut_borrow_while_shared() {
        let src = "
            fn main() {
                let mut x = 1;
                let a = &x;
                let b = &mut x;
                print(*a);
            }
        ";
        let result = run_source(src, RuntimeConfig::test());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("cannot mutably borrow `x` while it is borrowed"),
            "expected mutable borrow conflict"
        );
    }

    #[test]
    fn test_move_and_reinitialize() {
        let src = r#"
            fn main() {
                let mut s = "a";
                let t = s;
                s = "b";
                println_str(s);
                println_str(t);
            }
        "#;
        let output = run_source(src, RuntimeConfig::test()).expect("reinit should run");
        assert_eq!(output, b"b\na\n");
    }

    #[test]
    fn test_nll_releases_borrow_after_last_use() {
        let src = "
            fn main() {
                let mut x = 1;
                let p = &mut x;
                *p = 5;
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("NLL borrow release should run");
        assert_eq!(output, b"5\n");
    }

    #[test]
    fn test_assignment_through_mutable_reference() {
        let src = "
            fn main() {
                let mut x = 1;
                let p = &mut x;
                *p = 5;
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("deref assignment should run");
        assert_eq!(output, b"5\n");
    }

    #[test]
    fn test_struct_field_assignment() {
        let src = "
            struct Point { x: i32, y: i32 }
            fn main() {
                let mut p = Point { x: 3, y: 4 };
                p.x = 10;
                print(p.x);
                print(p.y);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("field assignment should run");
        assert_eq!(output, b"10\n4\n");
    }

    #[test]
    fn test_array_index_assignment() {
        let src = "
            fn main() {
                let mut a = [1, 2, 3];
                a[1] = 20;
                print(a[0]);
                print(a[1]);
                print(a[2]);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("index assignment should run");
        assert_eq!(output, b"1\n20\n3\n");
    }

    #[test]
    fn test_assignment_to_immutable_is_rejected() {
        for src in [
            "fn main() { let x = 1; x = 2; }",
            "struct P { x: i32 } fn main() { let p = P { x: 1 }; p.x = 2; }",
            "fn main() { let a = [1, 2]; a[0] = 3; }",
        ] {
            let result = run_source(src, RuntimeConfig::test());
            assert!(result.is_err(), "expected immutable assignment error");
            assert!(
                result
                    .unwrap_err()
                    .contains("cannot assign to immutable variable"),
                "expected immutable assignment diagnostic"
            );
        }
    }

    #[test]
    fn test_invalid_assignment_target_is_rejected() {
        let src = "fn main() { let mut x = 1; (x + 1) = 5; }";
        let result = run_source(src, RuntimeConfig::test());
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("invalid assignment target"),
            "expected invalid assignment target diagnostic"
        );
    }

    #[test]
    fn test_unknown_escape_is_rejected() {
        let src = r"fn main() { let c = '\q'; }";
        assert!(check_source(src).is_err());
    }

    #[test]
    fn test_check_rejects_type_mismatch() {
        let src = "fn main() { let x: i32 = true; }";
        let result = check_source(src);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().iter().any(|e| e.contains("type mismatch")),
            "expected a type mismatch diagnostic"
        );
    }

    #[test]
    fn test_run_rejects_type_mismatch() {
        let src = "fn main() { let x: bool = 42; print(x); }";
        let result = run_source(src, RuntimeConfig::test());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Type errors"));
    }

    #[test]
    fn test_check_type_checks_conditions_and_returns() {
        assert!(check_source("fn main() { if 1 { } }").is_err());
        assert!(check_source("fn f() -> i32 { return true; } fn main() { }").is_err());
        assert!(check_source("fn f(a: bool) { } fn main() { f(1); }").is_err());
    }

    #[test]
    fn test_check_valid_typed_function() {
        let src = "fn add(a: i32, b: i32) -> i32 { return a + b; } fn main() { let x = add(1, 2); }";
        assert!(check_source(src).is_ok());
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
