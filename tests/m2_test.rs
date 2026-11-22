//! M2 语言补全验收测试：循环值语义（M2-6）与模式匹配收尾（M2-5）。

mod common;

use common::assert_same_output;
use freebird::{check_source, check_source_with_warnings};

#[test]
fn if_let_enum_branches_match() {
    let source = r#"
enum Opt { None, Some(i64) }
fn main() {
    let a = Opt::Some(7);
    if let Opt::Some(x) = a { print(x); } else { print(0); }
    let b = Opt::None;
    if let Opt::Some(x) = b { print(x); } else { print(0); }
}
"#;
    assert_same_output(source, "if_let_enum", b"");
}

#[test]
fn if_let_else_if_chain_matches() {
    let source = r#"
enum E { A, B(i64), C }
fn main() {
    let x = E::B(5);
    if let E::A = x {
        print(1);
    } else if let E::B(v) = x {
        print(v);
    } else {
        print(3);
    }
}
"#;
    assert_same_output(source, "if_let_chain", b"");
}

#[test]
fn while_let_enum_matches() {
    let source = r#"
enum Opt { None, Some(i64) }
fn main() {
    let mut o = Opt::Some(3);
    while let Opt::Some(x) = o {
        print(x);
        o = Opt::None;
    }
    print(0);
}
"#;
    assert_same_output(source, "while_let_enum", b"");
}

#[test]
fn loop_break_value_matches() {
    let source = r#"
fn main() {
    let v = loop { break 42; };
    print(v);
    let mut i = 0;
    let w = loop {
        i = i + 1;
        if i >= 3 { break i * 10; }
    };
    print(w);
}
"#;
    assert_same_output(source, "loop_break_value", b"");
}

#[test]
fn nested_loop_break_value_matches() {
    let source = r#"
fn main() {
    let mut i = 0;
    let v = loop {
        i = i + 1;
        let w = loop { break i * 2; };
        if i >= 3 { break w + 1; }
    };
    print(v);
}
"#;
    assert_same_output(source, "nested_loop_break_value", b"");
}

#[test]
fn break_value_outside_loop_is_rejected() {
    let errors = check_source("fn main() { while true { break 1; } }")
        .expect_err("while break value should be rejected");
    assert!(
        errors
            .join("\n")
            .contains("`break value` is only supported inside `loop`"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn tuple_rest_pattern_matches() {
    let source = r#"
fn main() {
    let t = (1, 2, 3);
    match t {
        (a, ..) => print(a),
    }
    match t {
        (.., b) => print(b),
    }
}
"#;
    assert_same_output(source, "tuple_rest_pattern", b"");
}

#[test]
fn struct_rest_pattern_matches() {
    let source = r#"
struct P { x: i64, y: i64, z: i64 }
fn main() {
    let p = P { x: 1, y: 2, z: 3 };
    match p {
        P { x, .. } => print(x),
    }
}
"#;
    assert_same_output(source, "struct_rest_pattern", b"");
}

#[test]
fn unreachable_match_branch_warns() {
    let source = r#"
fn main() {
    let x = 1;
    match x {
        1 => print(1),
        1 => print(2),
        _ => print(3),
    }
}
"#;
    let (errors, warnings) = check_source_with_warnings(source);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("unreachable pattern")),
        "expected an unreachable-pattern warning, got {warnings:?}"
    );
}

#[test]
fn branch_after_wildcard_warns() {
    let source = r#"
fn main() {
    let x = 1;
    match x {
        _ => print(0),
        1 => print(1),
    }
}
"#;
    let (errors, warnings) = check_source_with_warnings(source);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("unreachable pattern")),
        "expected an unreachable-pattern warning, got {warnings:?}"
    );
}

