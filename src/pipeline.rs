use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use crate::borrowck::check_borrow;
use crate::codegen::stardust::FnInfo;
use crate::codegen::{optimize_mir, StardustGen};
use crate::frontend::parse;
use crate::hir::lower;
use crate::mir::MirGen;
use crate::typeck::{check_program_with_tags, check_program_with_warnings, TypeTable};
use crate::vm::config::RuntimeConfig;
use crate::vm::id::FnId;
use crate::vm::mir::MirInst;
use crate::vm::Vm;

/// 编译执行流水线：源码 → AST → HIR → MIR → VM 执行
/// 编译执行源码：解析后交给 `run_ast`。
pub fn run_source(source: &str, config: RuntimeConfig) -> Result<Vec<u8>, String> {
    let (ast, ast_errors) = parse(source);
    if !ast_errors.is_empty() {
        let msg: Vec<String> = ast_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Parse errors:\n{}", msg.join("\n")));
    }
    run_ast(ast, config)
}

/// 从文件加载并执行程序（支持 `mod name;` 递归加载同目录模块）。
pub fn run_file(path: &Path, config: RuntimeConfig) -> Result<Vec<u8>, String> {
    let ast = load_program(path)?;
    run_ast(ast, config)
}

/// 递归加载模块：`mod foo;` 读取同目录 `foo.fb`，其顶层函数以 `foo::name` 注册。
fn load_program(path: &Path) -> Result<crate::frontend::ast::Program, String> {
    let mut loading: Vec<std::path::PathBuf> = Vec::new();
    let mut pub_names: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    load_module(path, None, &mut loading, &mut pub_names)
}

fn load_module(
    path: &Path,
    prefix: Option<&str>,
    loading: &mut Vec<std::path::PathBuf>,
    pub_names: &mut std::collections::HashMap<String, std::collections::HashSet<String>>,
) -> Result<crate::frontend::ast::Program, String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("cannot resolve module `{}`: {e}", path.display()))?;
    if loading.contains(&canonical) {
        return Err(format!(
            "circular module dependency detected at `{}`",
            path.display()
        ));
    }
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read module `{}`: {e}", path.display()))?;
    let (mut parsed, errors) = parse(&source);
    if !errors.is_empty() {
        let msg: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        return Err(format!(
            "Parse errors in `{}`:\n{}",
            path.display(),
            msg.join("\n")
        ));
    }
    // 先收集本模块声明的类型名（用于改写类型引用），再消费 decls。
    let local_types = local_type_names(&parsed.decls);
    // 记录本模块的公开名字（`pub`），供跨模块 `use` 的可见性检查。
    if let Some(prefix) = prefix {
        let mut visible = std::collections::HashSet::new();
        for decl in &parsed.decls {
            match decl {
                crate::frontend::ast::Decl::Fn(f) if f.public => {
                    visible.insert(f.name.clone());
                }
                crate::frontend::ast::Decl::Struct(s) if s.public => {
                    visible.insert(s.name.clone());
                }
                crate::frontend::ast::Decl::Enum(e) if e.public => {
                    visible.insert(e.name.clone());
                }
                _ => {}
            }
        }
        pub_names.insert(prefix.to_string(), visible);
    }
    loading.push(canonical);
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut decls = Vec::new();
    for decl in parsed.decls.drain(..) {
        match decl {
            crate::frontend::ast::Decl::Mod { name, span } => {
                let sub_path = dir.join(format!("{name}.fb"));
                let sub_prefix = match prefix {
                    Some(p) => format!("{p}::{name}"),
                    None => name.clone(),
                };
                let sub = load_module(&sub_path, Some(&sub_prefix), loading, pub_names).map_err(|e| {
                    format!("while loading module `{name}` (declared at {span}): {e}")
                })?;
                decls.extend(sub.decls);
            }
            other => decls.push(apply_prefix(other, prefix, &local_types)),
        }
    }
    loading.pop();
    // 跨模块 `use mod::name;` 只能访问 `pub` 名字。
    for decl in &decls {
        if let crate::frontend::ast::Decl::Use(path, span) = decl
            && path.len() >= 2
        {
            let module = path[..path.len() - 1].join("::");
            let name = &path[path.len() - 1];
            if let Some(visible) = pub_names.get(&module)
                && !visible.contains(name)
            {
                return Err(format!(
                    "`{module}::{name}` is not `pub` (used at {span}); declare it `pub` in `{module}`"
                ));
            }
        }
    }
    Ok(crate::frontend::ast::Program { decls })
}

/// 收集模块顶层声明的类型名。
fn local_type_names(decls: &[crate::frontend::ast::Decl]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for decl in decls {
        match decl {
            crate::frontend::ast::Decl::Struct(s) => {
                names.insert(s.name.clone());
            }
            crate::frontend::ast::Decl::Enum(e) => {
                names.insert(e.name.clone());
            }
            _ => {}
        }
    }
    names
}

/// 给模块内声明加前缀，并把指向本模块类型的引用改写成 `mod::Name`。
fn apply_prefix(
    decl: crate::frontend::ast::Decl,
    prefix: Option<&str>,
    local: &std::collections::HashSet<String>,
) -> crate::frontend::ast::Decl {
    use crate::frontend::ast::Decl;
    let Some(prefix) = prefix else { return decl };
    let rename = |name: &str| -> String {
        if local.contains(name) {
            format!("{prefix}::{name}")
        } else {
            name.to_string()
        }
    };
    match decl {
        Decl::Struct(mut s) => {
            s.name = format!("{prefix}::{}", s.name);
            for f in &mut s.fields {
                f.ty = prefix_type(&f.ty, &rename);
            }
            Decl::Struct(s)
        }
        Decl::Enum(mut e) => {
            e.name = format!("{prefix}::{}", e.name);
            for v in &mut e.variants {
                if let Some(data) = &v.data {
                    v.data = Some(prefix_type(data, &rename));
                }
            }
            Decl::Enum(e)
        }
        Decl::Impl(mut i) => {
            i.type_name = rename(&i.type_name);
            for f in &mut i.fns {
                for p in &mut f.params {
                    p.ty = prefix_type(&p.ty, &rename);
                }
                if let Some(ret) = &f.return_type {
                    f.return_type = Some(prefix_type(ret, &rename));
                }
                if let Some(body) = &mut f.body {
                    prefix_block(body, &rename);
                }
            }
            Decl::Impl(i)
        }
        Decl::Fn(mut f) => {
            f.name = format!("{prefix}::{}", f.name);
            for p in &mut f.params {
                p.ty = prefix_type(&p.ty, &rename);
            }
            if let Some(ret) = &f.return_type {
                f.return_type = Some(prefix_type(ret, &rename));
            }
            if let Some(body) = &mut f.body {
                prefix_block(body, &rename);
            }
            Decl::Fn(f)
        }
        other => other,
    }
}

fn prefix_type(
    ty: &crate::frontend::ast::Type,
    rename: &impl Fn(&str) -> String,
) -> crate::frontend::ast::Type {
    use crate::frontend::ast::Type;
    match ty {
        Type::Path(segments, span) => {
            if segments.len() == 1 {
                Type::Path(vec![rename(&segments[0])], *span)
            } else {
                ty.clone()
            }
        }
        Type::Generic(base, args, span) => Type::Generic(
            Box::new(prefix_type(base, rename)),
            args.iter().map(|a| prefix_type(a, rename)).collect(),
            *span,
        ),
        Type::Ref(inner, mutable, span) => {
            Type::Ref(Box::new(prefix_type(inner, rename)), *mutable, *span)
        }
        Type::Tuple(types, span) => {
            Type::Tuple(types.iter().map(|t| prefix_type(t, rename)).collect(), *span)
        }
        Type::Array(inner, len, span) => {
            Type::Array(Box::new(prefix_type(inner, rename)), len.clone(), *span)
        }
        Type::FnPtr(params, ret, span) => Type::FnPtr(
            params.iter().map(|p| prefix_type(p, rename)).collect(),
            ret.as_ref().map(|r| Box::new(prefix_type(r, rename))),
            *span,
        ),
        other => other.clone(),
    }
}

fn prefix_block(block: &mut crate::frontend::ast::Block, rename: &impl Fn(&str) -> String) {
    for stmt in &mut block.stmts {
        match stmt {
            crate::frontend::ast::Stmt::Let { ty, init, .. } => {
                if let Some(t) = ty {
                    *t = prefix_type(t, rename);
                }
                if let Some(e) = init {
                    prefix_expr(e, rename);
                }
            }
            crate::frontend::ast::Stmt::Expr(e, _) => prefix_expr(e, rename),
            crate::frontend::ast::Stmt::Assign { target, value, .. } => {
                prefix_expr(target, rename);
                prefix_expr(value, rename);
            }
        }
    }
    if let Some(tail) = &mut block.tail {
        prefix_expr(tail, rename);
    }
}

fn prefix_expr(expr: &mut crate::frontend::ast::Expr, rename: &impl Fn(&str) -> String) {
    use crate::frontend::ast::{CallArg, Expr};
    match expr {
        Expr::Var(name, _) => {
            if let Some((ty, rest)) = name.split_once("::") {
                let renamed = rename(ty);
                if renamed != ty {
                    *name = format!("{renamed}::{rest}");
                }
            } else {
                let renamed = rename(name);
                if renamed != *name {
                    *name = renamed;
                }
            }
        }
        Expr::Deref(inner, _) | Expr::Unary(_, inner, _) => prefix_expr(inner, rename),
        Expr::Ref(inner, _, _) => prefix_expr(inner, rename),
        Expr::Binary(lhs, _, rhs, _) => {
            prefix_expr(lhs, rename);
            prefix_expr(rhs, rename);
        }
        Expr::Call { func, args, .. } => {
            prefix_expr(func, rename);
            for arg in args {
                match arg {
                    CallArg::Pos(e, _) | CallArg::Named(_, e, _) => prefix_expr(e, rename),
                }
            }
        }
        Expr::FieldAccess(inner, _, _) => prefix_expr(inner, rename),
        Expr::Index(a, b, _) => {
            prefix_expr(a, rename);
            prefix_expr(b, rename);
        }
        Expr::Tuple(elems, _) | Expr::Array(elems, _) => {
            for e in elems {
                prefix_expr(e, rename);
            }
        }
        Expr::Repeat(a, b, _) => {
            prefix_expr(a, rename);
            prefix_expr(b, rename);
        }
        Expr::StructLit { name, fields, .. } => {
            if name.len() == 1 {
                let renamed = rename(&name[0]);
                name[0] = renamed;
            }
            for (_, e) in fields {
                prefix_expr(e, rename);
            }
        }
        Expr::EnumLit { name, data, .. } => {
            if name.len() == 1 {
                let renamed = rename(&name[0]);
                name[0] = renamed;
            }
            if let Some(d) = data {
                prefix_expr(d, rename);
            }
        }
        Expr::If(cond, then_b, else_b, _) => {
            prefix_expr(cond, rename);
            prefix_block(then_b, rename);
            if let Some(b) = else_b {
                prefix_block(b, rename);
            }
        }
        Expr::Match(scrutinee, arms, _) => {
            prefix_expr(scrutinee, rename);
            for arm in arms {
                if let Some(g) = &mut arm.guard {
                    prefix_expr(g, rename);
                }
                prefix_expr(&mut arm.body, rename);
            }
        }
        Expr::Loop(block, _) | Expr::Unsafe(block, _) => prefix_block(block, rename),
        Expr::While(cond, body, _) => {
            prefix_expr(cond, rename);
            prefix_block(body, rename);
        }
        Expr::For { iterable, body, .. } => {
            prefix_expr(iterable, rename);
            prefix_block(body, rename);
        }
        Expr::Block(block, _) => prefix_block(block, rename),
        Expr::Labelled(_, inner, _) => prefix_expr(inner, rename),
        Expr::Break(v, _) | Expr::BreakLabel(_, v, _) => {
            if let Some(e) = v {
                prefix_expr(e, rename);
            }
        }
        Expr::Return(elems, _) => {
            for e in elems {
                prefix_expr(e, rename);
            }
        }
        Expr::As(inner, ty, _) => {
            prefix_expr(inner, rename);
            *ty = Box::new(prefix_type(ty, rename));
        }
        _ => {}
    }
}

/// 执行已解析的程序。
fn run_ast(ast: crate::frontend::ast::Program, config: RuntimeConfig) -> Result<Vec<u8>, String> {
    let mut config = config;
    // `mod name;` 需要文件级模块加载器（尚未实现），否则会被静默忽略。
    if ast.decls.iter().any(|d| matches!(d, crate::frontend::ast::Decl::Mod { .. })) {
        return Err(
            "Module declarations (`mod name;`) need the file-based module loader, which is not implemented yet (M2-3)."
                .to_string(),
        );
    }

    // 1.5 构建类型表
    let type_table = TypeTable::from_ast_decls(&ast.decls);

    // 2. HIR 降级 + 名称解析
    let (hir, hir_errors) = lower(&ast);
    if !hir_errors.is_empty() {
        let msg: Vec<String> = hir_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Resolve errors:\n{}", msg.join("\n")));
    }

    // 2.5 基础类型检查（同时取出内建枚举方法标签标注，供 MIR 生成使用）
    let (type_errors, method_tags) = check_program_with_tags(&hir, &type_table);
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

    // 2.7 `main` 上的运行时配置属性覆盖调用方配置。
    apply_runtime_attributes(&hir, &mut config);

    // 3. MIR 生成
    let mut generator = MirGen::new()
        .with_type_table(type_table)
        .with_method_variant_tags(method_tags);
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


/// 从 `main` 函数的属性读取运行时限制并覆盖配置。
///
/// 值已在类型检查阶段校验为非负整数；这里只做防御性解析。
fn apply_runtime_attributes(program: &crate::hir::types::HirProgram, config: &mut RuntimeConfig) {
    use crate::frontend::ast::AttributeArg;

    let main = program.decls.iter().find_map(|decl| match decl {
        crate::hir::types::HirDecl::Fn(f) if f.name == "main" => Some(f),
        _ => None,
    });
    let Some(main) = main else { return };

    for attr in &main.attrs {
        let value = attr.args.iter().find_map(|arg| match arg {
            AttributeArg::KeyValue(key, value) if key == &attr.name => Some(value.as_str()),
            AttributeArg::Value(value) => Some(value.as_str()),
            _ => None,
        });
        let Some(value) = value.and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        match attr.name.as_str() {
            "max_stack" => config.max_stack = value as usize,
            "max_call_depth" => config.max_call_depth = value as usize,
            "max_steps" => config.max_steps = value,
            "time_limit" => config.time_limit = Some(value),
            _ => {}
        }
    }
}

/// 检查源码（词法+语法+名称解析+类型/借用检查），同时返回警告。
///
/// 返回值第一项是错误，第二项是警告；有警告但无错误时程序仍可编译。
pub fn check_source_with_warnings(source: &str) -> (Vec<String>, Vec<String>) {
    let (ast, ast_errors) = parse(source);
    let mut all_errors: Vec<String> = ast_errors.iter().map(|e| e.to_string()).collect();
    let mut warnings: Vec<String> = Vec::new();

    let (hir, hir_errors) = lower(&ast);
    all_errors.extend(hir_errors.iter().map(|e| e.to_string()));

    if ast_errors.is_empty()
        && hir_errors.is_empty()
        && ast.decls.iter().any(|d| matches!(d, crate::frontend::ast::Decl::Mod { .. }))
    {
        all_errors.push(
            "Module declarations (`mod name;`) need the file-based module loader, which is not implemented yet (M2-3)."
                .to_string(),
        );
    }

    if ast_errors.is_empty() && hir_errors.is_empty() {
        let type_table = TypeTable::from_ast_decls(&ast.decls);
        let (type_errors, type_warnings) = check_program_with_warnings(&hir, &type_table);
        all_errors.extend(type_errors.iter().map(|e| e.to_string()));
        warnings.extend(type_warnings.iter().map(|w| w.to_string()));

        if type_errors.is_empty() {
            all_errors.extend(check_borrow(&hir).iter().map(|e| e.to_string()));
        }
    }

    (all_errors, warnings)
}

/// 检查源码（词法+语法+名称解析+基础类型检查，不执行）
pub fn check_source(source: &str) -> Result<(), Vec<String>> {
    let (errors, _warnings) = check_source_with_warnings(source);
    if errors.is_empty() { Ok(()) } else { Err(errors) }
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

    let (type_errors, method_tags) = check_program_with_tags(&hir, &type_table);
    if !type_errors.is_empty() {
        let msg: Vec<String> = type_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Type errors:\n{}", msg.join("\n")));
    }

    let borrow_errors = check_borrow(&hir);
    if !borrow_errors.is_empty() {
        let msg: Vec<String> = borrow_errors.iter().map(|e| e.to_string()).collect();
        return Err(format!("Borrow errors:\n{}", msg.join("\n")));
    }

    let mut generator = MirGen::new()
        .with_type_table(type_table)
        .with_method_variant_tags(method_tags);
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
        let output = run_source(src, RuntimeConfig::test())
            .expect("user function should override builtin");
        assert!(output.is_empty());
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
        let src = "fn main() { let x = 40 + 2; print(x); }";
        let output = run_source(src, RuntimeConfig::test()).expect("simple addition should run");
        assert_eq!(output, b"42\n");
    }

    #[test]
    fn test_run_with_return() {
        let src = "
            fn add(a: i32, b: i32) -> i32 {
                return a + b;
            }
            fn main() {
                let x = add(40, 2);
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("return should run");
        assert_eq!(output, b"42\n");
    }

    #[test]
    fn test_run_if_else() {
        let src = "
            fn main() {
                let x = if true { 1 } else { 2 };
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("if else should run");
        assert_eq!(output, b"1\n");
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
    fn test_read_line_trims_crlf_in_preloaded_input() {
        let src = "
            fn main() {
                let a = read_line();
                let b = read_line();
                print_str(a);
                print_char('\\n');
                print_str(b);
                print_char('\\n');
            }
        ";
        let mut config = RuntimeConfig::test();
        config.input = b"alpha\r\nbravo\r\n".to_vec();
        let output = run_source(src, config).expect("CRLF input should run");
        assert_eq!(output, b"alpha\nbravo\n");
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
    fn test_mutable_reference_writes_back_across_function_call() {
        let src = "
            fn inc(p: &mut i64) { *p = *p + 1; }
            fn twice(p: &mut i64) { inc(p); inc(p); }
            fn main() {
                let mut x = 10;
                twice(&mut x);
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test())
            .expect("reference passed to a function should write back");
        assert_eq!(output, b"12\n");
    }

    #[test]
    fn test_shadowed_variable_restores_outer_binding() {
        let src = "
            fn main() {
                let mut x = 1;
                {
                    let x = 5;
                    print(x);
                }
                let p = &mut x;
                *p = 9;
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test())
            .expect("shadowed variable should restore outer binding");
        assert_eq!(output, b"5\n9\n");
    }

    #[test]
    fn test_statement_after_block_without_semicolon() {
        let src = "
            fn main() {
                let mut x = 1;
                let p = &mut x;
                { print(0); }
                *p = 4;
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test())
            .expect("statement starting with * after a block should parse");
        assert_eq!(output, b"0\n4\n");
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
                print(fib(5));
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("fibonacci should run");
        assert_eq!(output, b"5\n");
    }

    #[test]
    fn test_while_loop() {
        let src = "
            fn main() {
                let mut x = 0;
                while x < 5 {
                    x = x + 1;
                }
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("while loop should run");
        assert_eq!(output, b"5\n");
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
                print(sum_squares(3, 4));
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("nested calls should run");
        assert_eq!(output, b"25\n");
    }

    #[test]
    fn test_struct_declaration_and_field() {
        let src = "
            struct Point { x: i32, y: i32 };
            fn main() {
                let p = Point { x: 3, y: 4 };
                print(p.x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("struct field should run");
        assert_eq!(output, b"3\n");
    }

    #[test]
    fn test_struct_construction() {
        let src = "
            struct Point { x: i32, y: i32 }
            fn main() {
                let p = Point { x: 3, y: 4 };
                print(p.x + p.y);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("struct construction should run");
        assert_eq!(output, b"7\n");
    }

    #[test]
    fn test_enum_declaration() {
        let src = "
            enum Option { None, Some(i32) }
            fn main() {
                let x = Option::Some(42);
                match x { Option::None => print(0), Option::Some(v) => print(v), }
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("enum declaration should run");
        assert_eq!(output, b"42\n");
    }

    #[test]
    fn test_stardust_macro() {
        let src = "
            fn main() {
                let x = stardust! { Push(3) Push(5) Add };
                print(x);
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("stardust macro should run");
        assert_eq!(output, b"8\n");
    }

    #[test]
    fn test_unsafe_block() {
        let src = "
            fn main() {
                unsafe {
                    let x = 42;
                    print(x);
                }
            }
        ";
        let output = run_source(src, RuntimeConfig::test()).expect("unsafe block should run");
        assert_eq!(output, b"42\n");
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
        let output = run_source(src, RuntimeConfig::test()).expect("extern decl should run");
        assert!(output.is_empty());
    }
}
