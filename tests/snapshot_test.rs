//! 诊断快照测试（M5-5）。
//!
//! 只记录稳定的 `code / level / line`，不记录可能随措辞变化的完整消息。
//! 更新快照：
//!
//! ```bash
//! UPDATE_SNAPSHOTS=1 cargo test --test snapshot_test
//! ```

use std::path::PathBuf;

use freebird::diagnostics::{DiagnosticLevel, collect_diagnostics};

const CASES: &[(&str, &str)] = &[
    ("type_mismatch", "fn main() { let x: i32 = true; }"),
    ("undefined_variable", "fn main() { print(y); }"),
    ("duplicate_let", "fn main() { let x = 1; let x = 2; }"),
    (
        "use_after_move",
        "fn main() { let s = \"hi\"; let t = s; print_str(s); }",
    ),
    (
        "non_exhaustive_match",
        "enum E { A, B }\nfn main() { let e = E::A; match e { E::A => print(1) } }",
    ),
    (
        "ffi_outside_unsafe",
        "extern \"stardust\" { #[stardust(id = 1)] fn f(x: i64) -> i64; }\nfn main() { print(f(1)); }",
    ),
    (
        "unknown_stardust_instruction",
        "fn main() { let x = stardust! { Bogus }; print(x); }",
    ),
];

fn level_name(level: &DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Error => "error",
        DiagnosticLevel::Warning => "warning",
        DiagnosticLevel::Note => "note",
        DiagnosticLevel::Help => "help",
    }
}

fn snapshot_text() -> String {
    let mut output = String::new();
    for (name, source) in CASES {
        output.push_str(&format!("== {name} ==\n"));
        for diagnostic in collect_diagnostics(source) {
            output.push_str(&format!(
                "{} {} {}\n",
                diagnostic.code,
                level_name(&diagnostic.level),
                diagnostic.span.line
            ));
        }
        output.push('\n');
    }
    output
}

#[test]
fn diagnostics_snapshot() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
        .join("diagnostics.txt");
    let actual = snapshot_text();

    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().expect("snapshot parent")).expect("create snapshots");
        std::fs::write(&path, &actual).expect("write snapshot");
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        actual, expected,
        "diagnostics snapshot changed; run `UPDATE_SNAPSHOTS=1 cargo test --test snapshot_test`"
    );
}
