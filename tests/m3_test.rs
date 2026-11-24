//! M3 后端测试：LLVM 后端（经 Stardust 工具链）与 VM 的输出一致性。
//!
//! LLVM 构建需要 Stardust 可执行文件与 clang。仓库自带
//! `stardust/stardust-linux-x86_64`，因此 CI 默认可用；两者缺失时测试会被跳过
//! （打印 skip 原因）而不是失败。

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use freebird::codegen::llvm::{LlvmOptions, compile_llvm, locate_clang, locate_stardust};
use freebird::{RuntimeConfig, run_source};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn llvm_available() -> bool {
    locate_stardust(None).is_ok() && locate_clang().is_ok()
}

fn llvm_skip() -> bool {
    if llvm_available() {
        false
    } else {
        eprintln!("skipping LLVM test: Stardust toolchain or clang not available");
        true
    }
}

fn run_vm(source: &str, input: &[u8]) -> Vec<u8> {
    let mut config = RuntimeConfig::test();
    config.input = input.to_vec();
    match run_source(source, config) {
        Ok(output) => output,
        Err(error) => panic!("VM failed: {error}"),
    }
}

/// 编译为可执行文件并运行；返回 stdout。
fn run_llvm(source: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let exe: PathBuf =
        std::env::temp_dir().join(format!("freebird-m3-{}-{id}", std::process::id()));
    let options = LlvmOptions {
        emit: freebird::codegen::llvm::LlvmEmit::Executable,
        output: Some(exe.clone()),
        stardust_bin: None,
        opt_level: 0,
    };
    compile_llvm(source, &options)?;

    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run executable: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input)
            .map_err(|e| format!("write stdin: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("wait executable: {e}"))?;
    let _ = std::fs::remove_file(&exe);
    if !output.status.success() {
        return Err(format!(
            "executable failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

fn assert_same_output(name: &str, source: &str, input: &[u8]) {
    if llvm_skip() {
        return;
    }
    let vm = run_vm(source, input);
    let llvm = run_llvm(source, input).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(
        vm,
        llvm,
        "{name}: VM and LLVM outputs differ\nvm:   {:?}\nllvm: {:?}",
        String::from_utf8_lossy(&vm),
        String::from_utf8_lossy(&llvm)
    );
}

// ─── M3-1：标量、控制流、函数、I/O ───

#[test]
fn llvm_arithmetic_and_comparison() {
    assert_same_output(
        "arithmetic",
        r#"fn main() {
            print(1 + 2 * 3); print_char('\n');
            print(10 / 3); print_char('\n');
            print(10 % 3); print_char('\n');
            print(-7 + 2); print_char('\n');
            let a = 3; let b = 4;
            print((a < b) as i64); print_char('\n');
            print((a == b) as i64); print_char('\n');
            print((a != b) as i64); print_char('\n');
        }"#,
        b"",
    );
}

#[test]
fn llvm_control_flow() {
    assert_same_output(
        "control flow",
        r#"fn main() {
            let mut i = 0;
            let mut sum = 0;
            while i < 5 { sum = sum + i; i = i + 1; }
            print(sum); print_char('\n');
            let mut j = 0;
            loop { if j == 3 { break; } j = j + 1; }
            print(j); print_char('\n');
            let arr = [10, 20, 30, 40];
            let mut total = 0;
            for value in arr { total = total + value; }
            print(total); print_char('\n');
            let mut k = 0;
            while k < 10 { k = k + 1; if k % 2 == 0 { continue; } print(k); }
            print_char('\n');
        }"#,
        b"",
    );
}

#[test]
fn llvm_functions_and_recursion() {
    assert_same_output(
        "functions",
        r#"fn add(a: i32, b: i32) -> i32 { a + b }
        fn divrem(a: i32, b: i32) -> (i32, i32) { (a / b, a % b) }
        fn fib(n: i32) -> i32 {
            if n <= 1 { return n; }
            return fib(n - 1) + fib(n - 2);
        }
        fn main() {
            print(add(20, 22)); print_char('\n');
            print(fib(10)); print_char('\n');
            let (q, r) = divrem(17, 5);
            print(q); print_char('\n');
            print(r); print_char('\n');
        }"#,
        b"",
    );
}

#[test]
fn llvm_print_variants() {
    assert_same_output(
        "print variants",
        r#"fn main() {
            print_int(42); print_char('\n');
            print_str("abc"); print_char('\n');
            println_str("def");
            print_char('Z'); print_char('\n');
        }"#,
        b"",
    );
}

#[test]
fn llvm_read_int() {
    assert_same_output(
        "read_int",
        r#"fn main() {
            let n = read_int();
            print(n * 2);
        }"#,
        b"21\n",
    );
}

#[test]
fn llvm_read_char() {
    assert_same_output(
        "read_char",
        r#"fn main() {
            let c = read_char();
            print_char(c);
        }"#,
        b"Z",
    );
}

#[test]
fn llvm_read_line() {
    assert_same_output(
        "read_line",
        r#"fn main() {
            let line = read_line();
            print_str(line);
        }"#,
        b"hello\n",
    );
}

#[test]
fn llvm_stardust_macro_arithmetic() {
    assert_same_output(
        "stardust macro",
        r#"fn main() {
            let v = stardust! {
                Push(3)
                Push(5)
                Add
                Dup
                Mul
            };
            print(v); print_char('\n');
        }"#,
        b"",
    );
}

// ─── M3-2：复合类型 ───

#[test]
fn llvm_structs_and_arrays() {
    assert_same_output(
        "structs and arrays",
        r#"struct Point { x: i64, y: i64 }
        fn main() {
            let mut p = Point { x: 3, y: 4 };
            p.x = 10;
            print(p.x + p.y);
            let mut arr = [1, 2, 3, 4];
            print(arr[2]);
            arr[1] = 20;
            print(arr[1]);
            let repeat = [7; 3];
            print(repeat[0] + repeat[2]);
        }"#,
        b"",
    );
}

#[test]
fn llvm_enums_and_match() {
    assert_same_output(
        "enums",
        r#"enum Shape { Circle(i64), Square(i64), Dot }
        fn area(s: Shape) -> i64 {
            match s {
                Shape::Circle(r) => 3 * r * r,
                Shape::Square(a) => a * a,
                Shape::Dot => 0,
            }
        }
        fn main() {
            print(area(Shape::Circle(2)));
            print(area(Shape::Square(3)));
            print(area(Shape::Dot));
            let s = Shape::Circle(5);
            match s {
                Shape::Circle(r) => print(r * 2),
                _ => print(0),
            }
        }"#,
        b"",
    );
}

#[test]
fn llvm_references_cross_function() {
    assert_same_output(
        "references",
        r#"fn bump(p: &mut i64) { *p = *p + 1; }
        fn main() {
            let mut n = 41;
            bump(&mut n);
            print(n);
            let x = 5;
            let r = &x;
            print(*r);
        }"#,
        b"",
    );
}

#[test]
fn llvm_vec_string_box() {
    assert_same_output(
        "containers",
        r#"fn main() {
            let mut v = vec![3, 1, 4];
            v.push(1);
            print(v.len());
            print(v.pop());
            v[0] = 30;
            print(v[0]);
            let mut s: String = String::new();
            s.push('F');
            s.push('B');
            print(s.len());
            print_str(s);
            let boxed = Box::new(6 * 7);
            print(*boxed);
            let text = "hello";
            print(text.len());
            print_str(text);
        }"#,
        b"",
    );
}

#[test]
fn llvm_multi_value_and_function_pointers() {
    assert_same_output(
        "multi value and fn ptr",
        r#"fn inc(x: i64) -> i64 { x + 1 }
        fn add(a: i64, b: i64) -> i64 { a + b }
        fn pair() -> (i64, i64) { (3, 4) }
        fn apply(f: fn(i64) -> i64, x: i64) -> i64 { f(x) }
        fn main() {
            let (a, b) = pair();
            print(a * 10 + b);
            let f = inc;
            print(f(41));
            let g: fn(i64, i64) -> i64 = add;
            print(g(20, 22));
            print(apply(inc, 1));
        }"#,
        b"",
    );
}

#[test]
fn llvm_option_and_result() {
    assert_same_output(
        "option and result",
        r#"fn inc(x: i64) -> i64 { x + 1 }
        fn main() {
            let a = Option::Some(7);
            print(a.unwrap());
            print(a.is_some() as i64);
            let b: Option<i64> = Option::None;
            print(b.unwrap_or(9));
            print(b.map(inc).is_none() as i64);
            let ok: Result<i64, bool> = Result::Ok(3);
            print(ok.unwrap());
            let bad: Result<i64, bool> = Result::Err(true);
            print(bad.unwrap_or(4));
            print(bad.map(inc).is_err() as i64);
        }"#,
        b"",
    );
}

#[test]
fn llvm_impl_methods() {
    assert_same_output(
        "impl methods",
        r#"struct Rect { w: i64, h: i64 }
        impl Rect {
            fn square(side: i64) -> Rect { Rect { w: side, h: side } }
            fn area(&self) -> i64 { self.w * self.h }
            fn grow(&mut self, by: i64) {
                self.w = self.w + by;
                self.h = self.h + 1;
            }
        }
        fn main() {
            let s = Rect::square(3);
            print(s.area());
            let mut r = Rect { w: 1, h: 2 };
            r.grow(4);
            print(r.w);
            print(r.h);
            print(r.area());
        }"#,
        b"",
    );
}

#[test]
fn llvm_unsafe_raw_stack() {
    assert_same_output(
        "unsafe raw stack",
        r#"fn main() {
            unsafe {
                push(1);
                push(2);
                swap();
                print(pop());
                print(pop());
                push(7);
                dup();
                print(pop());
                print(pop());
            }
        }"#,
        b"",
    );
}

// ─── M3-3：Stardust FFI ───

fn run_stardust_text(text: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let stardust =
        locate_stardust(None).map_err(|e| format!("Stardust toolchain unavailable: {e}"))?;
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("freebird-m3-sd-{}-{id}.sd", std::process::id()));
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    let mut child = Command::new(&stardust)
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("run Stardust: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input)
            .map_err(|e| format!("write stdin: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("wait Stardust: {e}"))?;
    let _ = std::fs::remove_file(&path);
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// 生成一个 Stardust 函数 `id`，函数体为 `Add`（两个参数相加）。
fn stardust_add_fn(id: u64) -> String {
    let pad = " ".repeat(id as usize);
    format!("\n{pad}:\n*\n{pad}:\n")
}

const FFI_ADD: &str = r#"
extern "stardust" {
    #[stardust(id = 7)]
    fn sd_add(a: i64, b: i64) -> i64;
}
fn main() {
    unsafe { print(sd_add(3, 5)); }
}
"#;

#[test]
fn ffi_stardust_codegen_and_execution() {
    if locate_stardust(None).is_err() {
        eprintln!("skipping FFI execution: Stardust toolchain not available");
        return;
    }
    let sd = freebird::build_stardust(FFI_ADD).expect("build stardust");
    // Call(7, 2, 1)：7 个空格 + ':'，随后 2 个空格 + ';'
    assert!(sd.contains(&format!("{}:\n", " ".repeat(7))));
    assert!(sd.contains(&format!("{};\n", " ".repeat(2))));
    let combined = format!("{sd}{}", stardust_add_fn(7));
    let out = run_stardust_text(&combined, b"").expect("run combined Stardust");
    assert_eq!(out, b"8\n");
}

#[test]
fn ffi_default_id_is_derived_from_name() {
    if locate_stardust(None).is_err() {
        eprintln!("skipping FFI execution: Stardust toolchain not available");
        return;
    }
    let id = freebird::codegen::ffi::default_stardust_id("sd_add");
    let source = r#"
extern "stardust" {
    fn sd_add(a: i64, b: i64) -> i64;
}
fn main() {
    unsafe { print(sd_add(3, 5)); }
}
"#;
    let sd = freebird::build_stardust(source).expect("build stardust");
    assert!(sd.contains(&format!("{}:\n", " ".repeat(id as usize))));
    let combined = format!("{sd}{}", stardust_add_fn(id));
    let out = run_stardust_text(&combined, b"").expect("run combined Stardust");
    assert_eq!(out, b"8\n");
}

#[test]
fn ffi_multi_value_return() {
    if locate_stardust(None).is_err() {
        eprintln!("skipping FFI execution: Stardust toolchain not available");
        return;
    }
    let source = r#"
extern "stardust" {
    #[stardust(id = 9)]
    fn sd_pair(a: i64) -> (i64, i64);
}
fn main() {
    unsafe {
        let (x, y) = sd_pair(10);
        print(x);
        print(y);
    }
}
"#;
    let sd = freebird::build_stardust(source).expect("build stardust");
    // 函数体：Dup(a, a)、Push(1)（6 空格 + '+'）、Add → [a, a+1]
    let pad = " ".repeat(9);
    let ext = format!("\n{pad}:\n +\n      +\n*\n{pad}:\n");
    let out = run_stardust_text(&format!("{sd}{ext}"), b"").expect("run combined Stardust");
    assert_eq!(out, b"10\n11\n");
}

#[test]
fn ffi_id_one_does_not_collide_with_main() {
    if locate_stardust(None).is_err() {
        eprintln!("skipping FFI execution: Stardust toolchain not available");
        return;
    }
    let source = r#"
extern "stardust" {
    #[stardust(id = 1)]
    fn sd_add(a: i64, b: i64) -> i64;
}
fn main() {
    unsafe { print(sd_add(20, 22)); }
}
"#;
    let sd = freebird::build_stardust(source).expect("build stardust");
    // 内部 main 被重命名为 2，FFI 仍使用 1。
    assert!(sd.contains(&format!("{}:\n", " ")));
    assert!(sd.contains(&format!("{}:\n", " ".repeat(2))));
    let ext = stardust_add_fn(1);
    let out = run_stardust_text(&format!("{sd}{ext}"), b"").expect("run combined Stardust");
    assert_eq!(out, b"42\n");
}

#[test]
fn ffi_requires_unsafe_block() {
    let source = r#"
extern "stardust" {
    #[stardust(id = 3)]
    fn sd_add(a: i64, b: i64) -> i64;
}
fn main() { print(sd_add(1, 2)); }
"#;
    let errors = freebird::check_source(source).expect_err("safe extern call must be rejected");
    assert!(
        errors.join("\n").contains("unsafe"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn ffi_runs_on_vm_with_explicit_error() {
    let error = run_source(FFI_ADD, RuntimeConfig::test()).unwrap_err();
    assert!(
        error.contains("FFI") && error.contains("VM"),
        "unexpected error: {error}"
    );
}

#[test]
fn ffi_duplicate_ids_are_rejected() {
    let source = r#"
extern "stardust" {
    #[stardust(id = 3)]
    fn a(x: i64) -> i64;
    #[stardust(id = 3)]
    fn b(x: i64) -> i64;
}
fn main() { unsafe { print(a(1) + b(2)); } }
"#;
    let error = freebird::build_stardust(source).expect_err("duplicate ids must fail");
    assert!(
        error.contains("both extern functions"),
        "unexpected error: {error}"
    );
}

#[test]
fn ffi_llvm_requires_external_definition() {
    if llvm_skip() {
        return;
    }
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let exe = std::env::temp_dir().join(format!("freebird-m3-ffi-{}-{id}", std::process::id()));
    let options = LlvmOptions {
        emit: freebird::codegen::llvm::LlvmEmit::Executable,
        output: Some(exe.clone()),
        stardust_bin: None,
        opt_level: 0,
    };
    // 没有链接外部 Stardust 函数：clang 必须报告未定义符号，而不是生成坏程序。
    let error = compile_llvm(FFI_ADD, &options).expect_err("missing extern definition must fail");
    assert!(
        error.contains("clang") || error.contains("undefined") || error.contains("sd_func"),
        "unexpected error: {error}"
    );
    let _ = std::fs::remove_file(&exe);
}

// ─── M3-4：stardust! 指令补全 ───

#[test]
fn stardust_macro_labels_and_jumps() {
    assert_same_output(
        "macro loop",
        r#"fn main() {
            let v = stardust! {
                Push(0)
                Mark(1)
                Dup
                Push(3)
                Lt
                Jump(2)
                UncondJump(3)
                Mark(2)
                Push(1)
                Add
                UncondJump(1)
                Mark(3)
            };
            print(v);
        }"#,
        b"",
    );
}

#[test]
fn stardust_macro_pick_and_dropn() {
    assert_same_output(
        "macro pick/dropn",
        r#"fn main() {
            let v = stardust! {
                Push(1)
                Push(2)
                Push(3)
                Push(0)
                Pick
                Push(1)
                DropN
                Push(2)
                Push(2)
                DropN
                Push(0)
                DropN
            };
            print(v);
        }"#,
        b"",
    );
}

#[test]
fn stardust_macro_num_out_and_char_out() {
    assert_same_output(
        "macro out",
        r#"fn main() {
            stardust! {
                Push(65)
                CharOut
                Push(42)
                NumOut
                Nop
            }
        }"#,
        b"",
    );
}

#[test]
fn stardust_macro_num_in() {
    assert_same_output(
        "macro num in",
        r#"fn main() {
            stardust! {
                NumIn
                NumOut
            }
        }"#,
        b"7\n",
    );
}

#[test]
fn stardust_macro_char_in() {
    assert_same_output(
        "macro char in",
        r#"fn main() {
            stardust! {
                CharIn
                CharOut
            }
        }"#,
        b"Z",
    );
}

#[test]
fn stardust_macro_shift_left() {
    assert_same_output(
        "macro shift left",
        r#"fn main() {
            stardust! {
                Push(1)
                Push(2)
                Push(3)
                ShiftL
                NumOut
            }
        }"#,
        b"",
    );
}

#[test]
fn stardust_macro_shift_right() {
    assert_same_output(
        "macro shift right",
        r#"fn main() {
            stardust! {
                Push(1)
                Push(2)
                Push(3)
                Push(4)
                ShiftR
                NumOut
            }
        }"#,
        b"",
    );
}

#[test]
fn stardust_macro_rotate_and_depth() {
    assert_same_output(
        "macro rotate",
        r#"fn main() {
            stardust! {
                Push(1)
                Push(2)
                Push(3)
                Rotate
                NumOut
            }
        }"#,
        b"",
    );
    assert_same_output(
        "macro depth",
        r#"fn main() {
            stardust! {
                Push(1)
                Push(2)
                Depth
                NumOut
            }
        }"#,
        b"",
    );
}

#[test]
fn stardust_macro_debug_and_nop() {
    assert_same_output(
        "macro debug",
        r#"fn main() {
            stardust! {
                Push(1)
                DumpStack
                DumpState
                Breakpoint
                Nop
                NumOut
            }
        }"#,
        b"",
    );
}

#[test]
fn stardust_macro_unknown_instruction_is_rejected() {
    let errors = freebird::check_source(r#"fn main() { let x = stardust! { Bogus }; print(x); }"#)
        .expect_err("unknown instruction must be rejected");
    assert!(
        errors.join("\n").contains("unknown stardust instruction"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn stardust_macro_raw_call() {
    if locate_stardust(None).is_err() {
        eprintln!("skipping raw Call execution: Stardust toolchain not available");
        return;
    }
    let source = r#"fn main() {
        stardust! {
            Push(3)
            Push(5)
            Call(7, 2, 1)
            NumOut
        }
    }"#;
    let sd = freebird::build_stardust(source).expect("build stardust");
    let combined = format!("{sd}{}", stardust_add_fn(7));
    let out = run_stardust_text(&combined, b"").expect("run combined Stardust");
    assert_eq!(out, b"8\n");

    let error = run_source(source, RuntimeConfig::test()).unwrap_err();
    assert!(
        error.contains("Call") && error.contains("VM"),
        "unexpected error: {error}"
    );
}

#[test]
fn llvm_unicode_output_matches() {
    assert_same_output(
        "unicode output",
        r#"fn main() {
            print_char('你');
            println_str("你好");
        }"#,
        b"",
    );
}

// ─── M6-2.1：LLVM 文件 I/O ───

#[test]
fn llvm_file_io_matches() {
    let path = std::env::temp_dir().join(format!("freebird-m6-fileio-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let source = format!(
        r#"fn main() {{
            file_write("{path}", "hello 世界\nline2");
            let s = file_read("{path}");
            print_str(s);
            print(s.len());
        }}"#,
        path = path.display()
    );
    assert_same_output("file I/O", &source, b"");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn llvm_file_read_missing_fails() {
    if llvm_skip() {
        return;
    }
    let missing =
        std::env::temp_dir().join(format!("freebird-m6-missing-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let source = format!(
        r#"fn main() {{ let s = file_read("{p}"); print_str(s); }}"#,
        p = missing.display()
    );
    assert!(
        run_llvm(&source, b"").is_err(),
        "reading a missing file should fail on the LLVM backend"
    );
}
