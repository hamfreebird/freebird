#[cfg(test)]
mod tests {
    use crate::frontend::parse;
    use crate::frontend::ast::*;

    fn parse_expr(source: &str) -> Expr {
        let wrapped = format!("fn _test() {{ {} }}", source);
        let (prog, errors) = parse(&wrapped);
        if !errors.is_empty() {
            eprintln!("Parse errors for '{source}':");
            for e in &errors { eprintln!("  {e}"); }
        }
        if let Some(Decl::Fn(f)) = prog.decls.first() {
            if let Some(ref body) = f.body {
                if let Some(ref tail) = body.tail {
                    return (**tail).clone();
                }
                if let Some(Stmt::Expr(e, _)) = body.stmts.first() {
                    return e.clone();
                }
            }
        }
        panic!("failed to parse expression: {source}");
    }

    fn parse_stmt(source: &str) -> Stmt {
        let wrapped = format!("fn _test() {{ {} }}", source);
        let (prog, errors) = parse(&wrapped);
        if !errors.is_empty() {
            eprintln!("Parse errors for '{source}':");
            for e in &errors { eprintln!("  {e}"); }
        }
        if let Some(Decl::Fn(f)) = prog.decls.first() {
            if let Some(ref body) = f.body {
                if let Some(s) = body.stmts.first() {
                    return s.clone();
                }
            }
        }
        panic!("failed to parse statement: {source}");
    }

    fn parse_decl(source: &str) -> Decl {
        let (prog, errors) = parse(source);
        if !errors.is_empty() {
            eprintln!("Parse errors for '{source}':");
            for e in &errors { eprintln!("  {e}"); }
        }
        prog.decls.into_iter().next().expect("no decl parsed")
    }

    fn parse_program(source: &str) -> (Program, Vec<crate::frontend::error::FrontendError>) {
        parse(source)
    }

    #[test]
    fn test_parse_integer_literal() {
        assert!(matches!(parse_expr("42"), Expr::Lit(Literal::Int(42, _), _)));
    }

    #[test]
    fn test_parse_bool_literal() {
        assert!(matches!(parse_expr("true"), Expr::Lit(Literal::Bool(true), _)));
        assert!(matches!(parse_expr("false"), Expr::Lit(Literal::Bool(false), _)));
    }

    #[test]
    fn test_parse_string_literal() {
        assert!(matches!(parse_expr("\"hello\""), Expr::Lit(Literal::String(s), _) if s == "hello"));
    }

    #[test]
    fn test_parse_variable() {
        assert!(matches!(parse_expr("foo"), Expr::Var(name, _) if name == "foo"));
    }

    #[test]
    fn test_parse_binary_op() {
        let e = parse_expr("1 + 2");
        assert!(matches!(e, Expr::Binary(_, BinOp::Add, _, _)));
    }

    #[test]
    fn test_parse_precedence() {
        // 1 + 2 * 3 → 1 + (2 * 3)
        let e = parse_expr("1 + 2 * 3");
        match e {
            Expr::Binary(lhs, BinOp::Add, rhs, _) => {
                assert!(matches!(*lhs, Expr::Lit(Literal::Int(1, _), _)));
                assert!(matches!(*rhs, Expr::Binary(_, BinOp::Mul, _, _)));
            }
            _ => panic!("unexpected AST: {e:?}"),
        }
    }

    #[test]
    fn test_parse_paren_expr() {
        assert!(matches!(parse_expr("(1)"), Expr::Lit(Literal::Int(1, _), _)));
    }

    #[test]
    fn test_parse_tuple() {
        assert!(matches!(parse_expr("(1, true)"), Expr::Tuple(_, _)));
    }

    #[test]
    fn test_parse_unit() {
        assert!(matches!(parse_expr("()"), Expr::Lit(Literal::Unit, _)));
    }

    #[test]
    fn test_parse_array() {
        assert!(matches!(parse_expr("[1, 2, 3]"), Expr::Array(_, _)));
    }

    #[test]
    fn test_parse_repeat_array() {
        assert!(matches!(parse_expr("[0; 5]"), Expr::Repeat(_, _, _)));
    }

    #[test]
    fn test_parse_block() {
        let e = parse_expr("{ 42 }");
        match e {
            Expr::Block(block, _) => {
                assert!(matches!(*block.tail.unwrap(), Expr::Lit(Literal::Int(42, _), _)));
            }
            _ => panic!("unexpected AST: {e:?}"),
        }
    }

    #[test]
    fn test_parse_if_expr() {
        let e = parse_expr("if true { 1 } else { 2 }");
        assert!(matches!(e, Expr::If(_, _, Some(_), _)));
    }

    #[test]
    fn test_parse_loop() {
        assert!(matches!(parse_expr("loop { break 42 }"), Expr::Loop(_, _)));
    }

    #[test]
    fn test_parse_while() {
        assert!(matches!(parse_expr("while true { }"), Expr::While(_, _, _)));
    }

    #[test]
    fn test_parse_return() {
        let e = parse_expr("return 1, 2");
        assert!(matches!(e, Expr::Return(vals, _) if vals.len() == 2));
    }

    #[test]
    fn test_parse_fn_call_positional() {
        let e = parse_expr("foo(x, y)");
        match e {
            Expr::Call { args, stack_pass: false, .. } => {
                assert_eq!(args.len(), 2);
            }
            _ => panic!("unexpected AST: {e:?}"),
        }
    }

    #[test]
    fn test_parse_fn_call_stack_pass() {
        let e = parse_expr("foo()");
        assert!(matches!(e, Expr::Call { stack_pass: true, .. }));
    }

    #[test]
    fn test_parse_ref() {
        assert!(matches!(parse_expr("&x"), Expr::Ref(_, false, _)));
        assert!(matches!(parse_expr("&mut x"), Expr::Ref(_, true, _)));
    }

    #[test]
    fn test_parse_deref() {
        assert!(matches!(parse_expr("*x"), Expr::Deref(_, _)));
    }

    #[test]
    fn test_parse_unsafe_block() {
        assert!(matches!(parse_expr("unsafe { }"), Expr::Unsafe(_, _)));
    }

    #[test]
    fn test_parse_let_stmt() {
        let s = parse_stmt("let x = 42;");
        assert!(matches!(s, Stmt::Let { .. }));
    }

    #[test]
    fn test_parse_let_mut() {
        let s = parse_stmt("let mut x = 42;");
        match s {
            Stmt::Let { mutable: true, .. } => {}
            _ => panic!("expected mutable let"),
        }
    }

    #[test]
    fn test_parse_assign() {
        let s = parse_stmt("x = 42;");
        assert!(matches!(s, Stmt::Assign { .. }));
    }

    #[test]
    fn test_parse_function() {
        let src = "fn add(a: i32, b: i32) -> i32 { return a + b; }";
        let d = parse_decl(src);
        match d {
            Decl::Fn(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
                assert!(f.return_type.is_some());
                assert!(f.body.is_some());
            }
            _ => panic!("expected Fn, got {d:?}"),
        }
    }

    #[test]
    fn test_parse_empty_function() {
        let src = "fn main() { }";
        let d = parse_decl(src);
        assert!(matches!(d, Decl::Fn(_)));
    }

    #[test]
    fn test_parse_struct() {
        let src = "struct Point { x: i32, y: i32 }";
        let d = parse_decl(src);
        match d {
            Decl::Struct(s) => {
                assert_eq!(s.name, "Point");
                assert_eq!(s.fields.len(), 2);
            }
            _ => panic!("expected Struct"),
        }
    }

    #[test]
    fn test_parse_enum() {
        let src = "enum Option { None, Some(i32) }";
        let d = parse_decl(src);
        match d {
            Decl::Enum(e) => {
                assert_eq!(e.name, "Option");
                assert_eq!(e.variants.len(), 2);
            }
            _ => panic!("expected Enum"),
        }
    }

    #[test]
    fn test_parse_extern() {
        let src = "extern \"stardust\" { fn add(a: i32, b: i32) -> i32; }";
        let d = parse_decl(src);
        assert!(matches!(d, Decl::Extern(_)));
    }

    #[test]
    fn test_parse_use() {
        let src = "use std::vec::Vec;";
        let d = parse_decl(src);
        assert!(matches!(d, Decl::Use(_, _)));
    }

    #[test]
    fn test_parse_stardust_macro() {
        let e = parse_expr("stardust! { Push(3) Dup Add }");
        assert!(matches!(e, Expr::StardustMacro(_, _)));
    }

    #[test]
    fn test_parse_multi_segment_path() {
        let e = parse_expr("a::b::c");
        match e {
            Expr::EnumLit { name, variant, data, .. } => {
                assert_eq!(name, vec!["a".to_string(), "b".to_string()]);
                assert_eq!(variant, "c");
                assert!(data.is_none());
            }
            other => panic!("expected EnumLit, got {other:?}"),
        }
    }

    #[test]
    fn test_stardust_immediate_must_be_integer() {
        let (_, errors) = parse_program("fn main() { stardust! { Ret(x) } }");
        assert!(!errors.is_empty());
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.kind, crate::frontend::error::FrontendErrorKind::InvalidNumber(_))),
            "{errors:?}"
        );
    }

    #[test]
    fn test_parse_field_access() {
        let e = parse_expr("point.x");
        assert!(matches!(e, Expr::FieldAccess(_, f, _) if f == "x"));
    }

    #[test]
    fn test_parse_vec_macro() {
        let e = parse_expr("vec![1, 2, 3]");
        match e {
            Expr::Array(elems, _) => assert_eq!(elems.len(), 3),
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_generic_type() {
        let d = parse_decl("fn f(v: Vec<i32>) { }");
        let Decl::Fn(f) = d else {
            panic!("expected Fn");
        };
        assert!(matches!(
            &f.params[0].ty,
            Type::Generic(base, args, _)
                if matches!(**base, Type::Path(_, _)) && args.len() == 1
        ));
    }

    #[test]
    fn test_parse_index() {
        let e = parse_expr("arr[0]");
        assert!(matches!(e, Expr::Index(_, _, _)));
    }

    #[test]
    fn test_parse_match() {
        let e = parse_expr("match x { 1 => 2, _ => 3 }");
        match e {
            Expr::Match(_, arms, _) => assert_eq!(arms.len(), 2),
            _ => panic!("expected Match"),
        }
    }

    #[test]
    fn test_parse_strange_nested() {
        // 更复杂的嵌套表达式
        let e = parse_expr("if x > 0 { x } else { -x }");
        match e {
            Expr::If(_, _, Some(_), _) => {}
            _ => panic!("unexpected AST: {e:?}"),
        }
    }
}
