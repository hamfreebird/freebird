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
fn generic_struct_matches() {
    let source = r#"
struct Pair<T> { a: T, b: T }
fn main() {
    let p: Pair<i64> = Pair { a: 1, b: 2 };
    print(p.a);
    print(p.b);
}
"#;
    assert_same_output(source, "generic_struct", b"");
}

#[test]
fn generic_function_matches() {
    let source = r#"
fn id<T>(x: T) -> T { x }
fn main() {
    print(id(7));
    print(id(true) as i64);
}
"#;
    assert_same_output(source, "generic_fn", b"");
}

#[test]
fn generic_enum_matches() {
    let source = r#"
enum Maybe<T> { Nothing, Just(T) }
fn main() {
    let m: Maybe<i64> = Maybe::Just(5);
    match m {
        Maybe::Just(x) => print(x),
        Maybe::Nothing => print(0),
    }
    let n: Maybe<i64> = Maybe::Nothing;
    match n {
        Maybe::Just(x) => print(x),
        Maybe::Nothing => print(9),
    }
}
"#;
    assert_same_output(source, "generic_enum", b"");
}

#[test]
fn generic_function_with_generic_struct_matches() {
    let source = r#"
struct Pair<T> { a: T, b: T }
fn first<T>(p: Pair<T>) -> T { p.a }
fn main() {
    let p: Pair<i64> = Pair { a: 3, b: 4 };
    print(first(p));
}
"#;
    assert_same_output(source, "generic_combined", b"");
}

#[test]
fn generic_struct_field_mismatch_is_rejected() {
    let source = r#"
struct Pair<T> { a: T, b: T }
fn main() {
    let p = Pair { a: 1, b: true };
    print(p.a);
}
"#;
    let errors = check_source(source).expect_err("generic field mismatch should be rejected");
    assert!(
        errors.join("\n").contains("type mismatch"),
        "unexpected diagnostics: {errors:?}"
    );
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
fn result_methods_match() {
    let source = r#"
fn main() {
    let ok: Result<i64, bool> = Result::Ok(7);
    print(ok.is_ok() as i64);
    print(ok.is_err() as i64);
    print(ok.unwrap());
    print(ok.unwrap_or(0));

    let bad: Result<i64, bool> = Result::Err(true);
    print(bad.is_ok() as i64);
    print(bad.is_err() as i64);
    print(bad.unwrap_or(11));
}
"#;
    assert_same_output(source, "result_methods", b"");
}

#[test]
fn result_unwrap_err_fails_on_both_backends() {
    let source = r#"
fn main() {
    let bad: Result<i64, bool> = Result::Err(true);
    print(bad.unwrap());
}
"#;
    assert!(
        matches!(run_vm(source, b""), Run::Failed(_)),
        "VM should reject unwrap on Err"
    );
    assert!(
        matches!(run_stardust(source, "result_unwrap_err", b""), Run::Failed(_)),
        "Stardust should reject unwrap on Err"
    );
}

#[test]
fn generic_option_payloads_match() {
    let source = r#"
struct Pair { a: i64, b: i64 }
fn main() {
    let c: Option<char> = Option::Some('z');
    print(c.unwrap() as i64);

    let s: Option<String> = Option::Some(String::new());
    match s {
        Option::Some(v) => print(v.len()),
        Option::None => print(0),
    }

    let p = Pair { a: 1, b: 2 };
    let op: Option<Pair> = Option::Some(p);
    match op {
        Option::Some(q) => print(q.a + q.b),
        Option::None => print(0),
    }

    let inner: Option<i64> = Option::Some(4);
    let outer: Option<Option<i64>> = Option::Some(inner);
    match outer {
        Option::Some(x) => print(x.unwrap()),
        Option::None => print(0),
    }
}
"#;
    assert_same_output(source, "generic_option_payloads", b"");
}

#[test]
fn option_unwrap_or_type_mismatch_is_rejected() {
    let errors = check_source(
        "fn main() { let o: Option<i64> = Option::Some(1); print(o.unwrap_or(true)); }",
    )
    .expect_err("unwrap_or with mismatched payload should be rejected");
    assert!(
        errors.join("\n").contains("type mismatch"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn result_payload_type_mismatch_is_rejected() {
    let errors = check_source("fn main() { let r: Result<i64, bool> = Result::Ok(true); }")
        .expect_err("mismatched Ok payload should be rejected");
    assert!(
        errors.join("\n").contains("type mismatch"),
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


#[test]
fn option_result_conversions_match() {
    let source = r#"
fn main() {
    let ok: Result<i64, bool> = Result::Ok(7);
    let o = ok.ok();
    print(o.is_some() as i64);
    print(o.unwrap());

    let bad: Result<i64, bool> = Result::Err(true);
    let e = bad.err();
    print(e.is_some() as i64);
    print(e.unwrap() as i64);

    let none: Option<i64> = Option::None;
    let r = none.ok_or(5);
    print(r.is_err() as i64);
    print(r.unwrap_or(9));

    let some: Option<i64> = Option::Some(3);
    let r2 = some.ok_or(5);
    print(r2.is_ok() as i64);
    print(r2.unwrap());
}
"#;
    assert_same_output(source, "option_result_conversions", b"");
}

#[test]
fn ok_or_argument_type_is_checked() {
    let errors = check_source(
        "fn main() { let o: Option<i64> = Option::None; let r: Result<i64, bool> = o.ok_or(1); }",
    )
    .expect_err("ok_or with mismatched error type should be rejected");
    assert!(
        errors.join("\n").contains("type mismatch"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn int_to_str_matches() {
    let source = r#"
fn main() {
    print_str(int_to_str(0));
    println_str(int_to_str(-42));
    println_str(int_to_str(1234));
}
"#;
    assert_same_output(source, "int_to_str", b"");
}

#[test]
fn str_to_int_matches() {
    let source = r#"
fn main() {
    match str_to_int("1234") {
        Option::Some(v) => print(v),
        Option::None => print(-1),
    }
    match str_to_int("-42") {
        Option::Some(v) => print(v),
        Option::None => print(-2),
    }
    match str_to_int("12x") {
        Option::Some(v) => print(v),
        Option::None => print(-3),
    }
    match str_to_int("") {
        Option::Some(v) => print(v),
        Option::None => print(-4),
    }
}
"#;
    assert_same_output(source, "str_to_int", b"");
}
#[test]
fn impl_methods_and_associated_functions_match() {
    let source = r#"
struct Rect { w: i64, h: i64 }

impl Rect {
    fn square(side: i64) -> Rect {
        Rect { w: side, h: side }
    }
    fn area(self) -> i64 {
        self.w * self.h
    }
    fn widen(self, by: i64) -> Rect {
        Rect { w: self.w + by, h: self.h }
    }
}

fn main() {
    let s = Rect::square(3);
    print(s.area());
    let w = s.widen(2);
    print(w.area());
}
"#;
    assert_same_output(source, "impl_methods", b"");
}

#[test]
fn shared_reference_receiver_matches() {
    let source = r#"
struct Rect { w: i64, h: i64 }
impl Rect {
    fn area(&self) -> i64 { self.w * self.h }
}
fn main() {
    let r = Rect { w: 2, h: 3 };
    print(r.area());
}
"#;
    assert_same_output(source, "ref_receiver", b"");
}

#[test]
fn mutable_receiver_matches() {
    let source = r#"
struct Rect { w: i64, h: i64 }
impl Rect {
    fn grow(&mut self, by: i64) {
        self.w = self.w + by;
        self.h = self.h + 1;
    }
    fn area(&self) -> i64 { self.w * self.h }
}
fn main() {
    let mut r = Rect { w: 1, h: 2 };
    r.grow(4);
    print(r.w);
    print(r.h);
    print(r.area());
}
"#;
    assert_same_output(source, "mut_receiver", b"");
}
#[test]
fn zero_return_function_pointer_matches() {
    let source = r#"
fn noop(x: i64) { print(x); }
fn main() {
    let g: fn(i64) = noop;
    g(7);
}
"#;
    assert_same_output(source, "fnptr_zero_return", b"");
}

#[test]
fn multi_return_function_pointer_matches() {
    let source = r#"
fn pair() -> (i64, i64) { (3, 4) }
fn main() {
    let h: fn() -> (i64, i64) = pair;
    let (a, b) = h();
    print(a);
    print(b);
}
"#;
    assert_same_output(source, "fnptr_multi_return", b"");
}

#[test]
fn option_and_result_map_match() {
    let source = r#"
fn inc(x: i64) -> i64 { x + 1 }
fn main() {
    let o: Option<i64> = Option::Some(7);
    let m = o.map(inc);
    print(m.is_some() as i64);
    print(m.unwrap());

    let n: Option<i64> = Option::None;
    print(n.map(inc).is_none() as i64);

    let r: Result<i64, bool> = Result::Ok(3);
    let mr = r.map(inc);
    print(mr.is_ok() as i64);
    print(mr.unwrap());

    let e: Result<i64, bool> = Result::Err(true);
    print(e.map(inc).is_err() as i64);
}
"#;
    assert_same_output(source, "map_combinator", b"");
}

#[test]
fn module_declaration_needs_file_path() {
    let errors = check_source("mod foo;\nfn main() { print(1); }")
        .expect_err("in-memory sources cannot load sibling modules");
    assert!(
        errors.join("\n").contains("require a file path"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn module_loader_and_use_alias_match() {
    let dir = std::env::temp_dir().join(format!("freebird_m23_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("util.fb"), "pub fn double(x: i64) -> i64 { x * 2 }\n")
        .expect("write module");
    let main_path = dir.join("main.fb");
    std::fs::write(
        &main_path,
        "mod util;\nuse util::double;\nfn main() { print(double(21)); }\n",
    )
    .expect("write main");
    let out = freebird::run_file(&main_path, freebird::RuntimeConfig::test())
        .expect("module program should run");
    assert_eq!(String::from_utf8_lossy(&out), "42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn circular_module_dependency_is_reported() {
    let dir = std::env::temp_dir().join(format!("freebird_m23c_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("a.fb"), "mod b;\nfn main() { print(1); }\n").expect("write a");
    std::fs::write(dir.join("b.fb"), "mod a;\n").expect("write b");
    let err = freebird::run_file(&dir.join("a.fb"), freebird::RuntimeConfig::test())
        .expect_err("circular modules should be rejected");
    assert!(
        err.contains("circular module dependency"),
        "unexpected error: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsafe_raw_stack_matches() {
    let source = r#"
fn main() {
    unsafe {
        push(1);
        push(2);
        swap();
        print(pop());
        print(pop());
    }
}
"#;
    assert_same_output(source, "raw_stack", b"");
}

#[test]
fn unsafe_raw_dup_matches() {
    let source = r#"
fn main() {
    unsafe {
        push(7);
        dup();
        print(pop());
        print(pop());
    }
}
"#;
    assert_same_output(source, "raw_dup", b"");
}

#[test]
fn unsafe_raw_roll_matches() {
    let source = r#"
fn main() {
    unsafe {
        // roll(+n)：把从栈顶数位置 n 的元素提到栈顶。
        push(1); push(2); push(3); push(4);
        roll(2);
        print(pop());
        print(pop());
        print(pop());
        print(pop());
        // roll(n) 与 roll(-n) 互逆（readme §4.2.3）。
        push(1); push(2); push(3); push(4);
        roll(2);
        roll(-2);
        print(pop());
        print(pop());
        print(pop());
        print(pop());
        // roll(0) 是 no-op。
        push(7);
        roll(0);
        print(pop());
    }
}
"#;
    assert_same_output(source, "raw_roll", b"");
}

#[test]
fn unsafe_raw_stack_grows_matches() {
    let source = r#"
fn main() {
    let mut i = 0;
    unsafe {
        while i < 100 {
            push(i);
            i = i + 1;
        }
        // 栈（底→顶）为 0..99：跨过初始容量 64 后仍可 roll。
        roll(99);
        print(pop());
        roll(-98);
        print(pop());
        print(pop());
    }
}
"#;
    assert_same_output(source, "raw_grow", b"");
}

#[test]
fn unsafe_raw_roll_out_of_range() {
    let source = r#"
fn main() {
    unsafe {
        push(1);
        roll(1);
    }
}
"#;
    match run_vm(source, b"") {
        Run::Failed(error) => {
            assert!(error.contains("out of range"), "unexpected error: {error}")
        }
        Run::Ok(_) => panic!("roll beyond the raw stack must fail"),
    }
}

#[test]
fn raw_stack_builtins_require_unsafe() {
    let errors = check_source("fn main() { push(1); }")
        .expect_err("raw stack builtins should require an unsafe block");
    assert!(
        errors.join("\n").contains("outside an `unsafe` block"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn module_local_types_are_prefixed() {
    let dir = std::env::temp_dir().join(format!("freebird_m2x2_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(
        dir.join("util.fb"),
        "struct Point { x: i64, y: i64 }\n\npub fn make(x: i64, y: i64) -> Point {\n    Point { x: x, y: y }\n}\n\npub fn sum(p: Point) -> i64 { p.x + p.y }\n",
    )
    .expect("write module");
    let main_path = dir.join("main.fb");
    std::fs::write(
        &main_path,
        "mod util;\nuse util::make;\nuse util::sum;\n\nfn main() { print(sum(make(3, 4))); }\n",
    )
    .expect("write main");
    let out = freebird::run_file(&main_path, freebird::RuntimeConfig::test())
        .expect("module-local types should work");
    assert_eq!(String::from_utf8_lossy(&out), "7\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn private_module_items_are_not_importable() {
    let dir = std::env::temp_dir().join(format!("freebird_m2x3_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(
        dir.join("util.fb"),
        "pub fn double(x: i64) -> i64 { x * 2 }\nfn secret(x: i64) -> i64 { x }\n",
    )
    .expect("write module");
    let main_path = dir.join("main.fb");
    std::fs::write(&main_path, "mod util;\nuse util::secret;\nfn main() { print(secret(1)); }\n")
        .expect("write main");
    let err = freebird::run_file(&main_path, freebird::RuntimeConfig::test())
        .expect_err("non-pub items must not be importable");
    assert!(err.contains("is not `pub`"), "unexpected error: {err}");
    // 同一模块里 `pub` 的名字可以使用。
    std::fs::write(&main_path, "mod util;\nuse util::double;\nfn main() { print(double(21)); }\n")
        .expect("rewrite main");
    let out = freebird::run_file(&main_path, freebird::RuntimeConfig::test())
        .expect("pub items should be importable");
    assert_eq!(String::from_utf8_lossy(&out), "42\n");
    let _ = std::fs::remove_dir_all(&dir);
}
