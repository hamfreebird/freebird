//! M1-1：VM 与 Stardust 后端差分测试。
//!
//! 自动遍历 `tests/programs/*.fb` 与 `examples/*.fb`，对每个程序分别在 VM 和
//! 自带 Stardust 解释器上执行，比较：
//!
//! * stdout 必须逐字节一致；
//! * 成功/失败状态必须一致（错误消息由解释器决定，不比较文本）。
//!
//! 若同名 `.in` 文件存在，则作为标准输入；否则使用空输入。新增 fixture 会
//! 自动进入本测试，无需手工注册。

mod common;

use common::{assert_same, fb_files, run_stardust, run_vm, Run};

#[test]
fn differential_fixtures() {
    let mut files = fb_files("tests/programs");
    files.extend(fb_files("examples"));
    assert!(!files.is_empty(), "no .fb fixtures found");

    for path in files {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unnamed");
        assert_same(name, &path);
    }
}

#[test]
fn differential_runtime_errors() {
    // 两端都必须以非零状态拒绝这些程序；消息文本由运行时决定。
    let cases: &[(&str, &str)] = &[
        (
            "division_by_zero",
            "fn main() { let a = 10; let b = 0; print(a / b); }",
        ),
        (
            "index_out_of_bounds",
            "fn main() { let a = [1, 2, 3]; print(a[5]); }",
        ),
        (
            "index_store_out_of_bounds",
            "fn main() { let a = [1, 2, 3]; a[5] = 9; print(a[0]); }",
        ),
        (
            "negative_index",
            "fn main() { let a = [1, 2, 3]; print(a[-1]); }",
        ),
        (
            "negative_index_store",
            "fn main() { let a = [1, 2, 3]; a[-1] = 9; print(a[0]); }",
        ),
        (
            "empty_array_index",
            "fn main() { let a: [i64; 0] = []; print(a[0]); }",
        ),
        (
            "file_read_missing",
            "fn main() { let s = file_read(\"/nonexistent/freebird-test.txt\"); print_str(s); }",
        ),
    ];

    for (name, source) in cases {
        let vm = run_vm(source, &[]);
        let sd = run_stardust(source, &format!("error_{name}"), &[]);
        assert!(
            matches!(vm, Run::Failed(_)),
            "{name}: VM should fail, got {vm:?}"
        );
        assert!(
            matches!(sd, Run::Failed(_)),
            "{name}: Stardust should fail, got {sd:?}"
        );
    }
}
