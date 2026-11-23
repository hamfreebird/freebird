//! M2 语言补全验收测试：循环值语义（M2-6）与模式匹配收尾（M2-5）。

mod common;

use common::{assert_same_output, run_stardust, run_vm, Run};
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
fn loop_labels_break_continue_match() {
    let source = r#"
fn main() {
    let mut i = 0;
    let mut sum = 0;
    'outer: loop {
        i = i + 1;
        let mut j = 0;
        loop {
            j = j + 1;
            if i >= 3 { break 'outer; }
            if j > 2 { continue 'outer; }
            sum = sum + 1;
        }
    }
    print(sum);
}
"#;
    assert_same_output(source, "loop_labels", b"");
}

#[test]
fn loop_label_break_value_matches() {
    let source = r#"
fn main() {
    let mut i = 0;
    let v = 'outer: loop {
        i = i + 1;
        loop { break 'outer i * 10; }
    };
    print(v);
}
"#;
    assert_same_output(source, "loop_label_value", b"");
}

#[test]
fn unknown_loop_label_is_rejected() {
    let errors = check_source("fn main() { loop { break 'missing; } }")
        .expect_err("unknown label should be rejected");
    assert!(
        errors.join("\n").contains("unknown loop label"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn loop_labels_on_while_and_for_match() {
    let source = r#"
fn main() {
    let mut sum = 0;
    'outer: for x in [1, 2, 3] {
        if x == 2 { continue 'outer; }
        sum = sum + x;
    }
    let mut i = 0;
    'w: while i < 3 {
        i = i + 1;
        if i == 2 { continue 'w; }
        sum = sum + i;
    }
    print(sum);
}
"#;
    assert_same_output(source, "loop_labels_while_for", b"");
}

#[test]
fn function_pointer_values_match() {
    let source = r#"
fn inc(x: i64) -> i64 { x + 1 }
fn dec(x: i64) -> i64 { x - 1 }
fn add(a: i64, b: i64) -> i64 { a + b }
fn getval() -> i64 { 42 }
fn main() {
    let f = inc;
    print(f(1));
    let g: fn(i64) -> i64 = dec;
    print(g(10));
    let h = add;
    print(h(1, 2));
    let z = getval;
    print(z());
}
"#;
    assert_same_output(source, "fn_pointer_values", b"");
}

#[test]
fn function_pointer_parameter_and_return_match() {
    let source = r#"
fn inc(x: i64) -> i64 { x + 1 }
fn apply(f: fn(i64) -> i64, x: i64) -> i64 { f(x) }
fn get() -> fn(i64) -> i64 { inc }
fn main() {
    print(apply(inc, 41));
    let f = get();
    print(f(1));
}
"#;
    assert_same_output(source, "fn_pointer_param_return", b"");
}

#[test]
fn function_pointer_signature_mismatch_is_rejected() {
    let source = r#"
fn add(a: i64, b: i64) -> i64 { a + b }
fn main() {
    let f: fn(i64) -> i64 = add;
    print(f(1));
}
"#;
    let errors = check_source(source).expect_err("signature mismatch should be rejected");
    assert!(
        errors.join("\n").contains("type mismatch"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn option_methods_match() {
    let source = r#"
fn main() {
    let a = Option::Some(7);
    print(a.is_some() as i64);
    print(a.unwrap());
    let b = Option::None;
    print(b.is_none() as i64);
    print(b.unwrap_or(42));
}
"#;
    assert_same_output(source, "option_methods", b"");
}

#[test]
fn math_builtins_match() {
    let source = r#"
fn main() {
    print(abs(-3));
    print(abs(8));
    print(min(4, 9));
    print(max(4, 9));
    print(min(-2, 5));
    print(max(-2, 5));
}
"#;
    assert_same_output(source, "math_builtins", b"");
}

#[test]
fn container_methods_match() {
    let source = r#"
fn main() {
    let mut v = Vec::new();
    print(v.is_empty() as i64);
    v.push(1);
    print(v.is_empty() as i64);
    v.clear();
    print(v.is_empty() as i64);
    let mut s = String::new();
    print(s.is_empty() as i64);
    s.push('a');
    print(s.is_empty() as i64);
    s.clear();
    print(s.is_empty() as i64);
}
"#;
    assert_same_output(source, "container_methods", b"");
}

#[test]
fn option_unwrap_none_fails_on_both_backends() {
    let source = r#"
fn main() {
    let b = Option::None;
    print(b.unwrap());
}
"#;
    assert!(
        matches!(run_vm(source, b""), Run::Failed(_)),
        "VM should reject unwrap on None"
    );
    assert!(
        matches!(run_stardust(source, "option_unwrap_none", b""), Run::Failed(_)),
        "Stardust should reject unwrap on None"
    );
}

#[test]
fn result_unwrap_is_explicitly_unsupported() {
    let errors = check_source("fn main() { let r = Result::Ok(1); print(r.unwrap()); }")
        .expect_err("Result::unwrap should be rejected");
    assert!(
        errors.join("\n").contains("Result::unwrap"),
        "unexpected diagnostics: {errors:?}"
    );
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

