//! VM / Stardust 差分测试的共享辅助。
//!
//! 这些函数被 `tests/differential_test.rs` 与 `tests/m1_test.rs` 复用；
//! `tests/common/mod.rs` 不会被 Cargo 当作独立测试目标。

#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use freebird::{RuntimeConfig, build_stardust, run_source};

pub const INTERPRETER: &str = "stardust/stardust-linux-x86_64";

#[derive(Debug)]
pub enum Run {
    Ok(Vec<u8>),
    Failed(String),
}

/// 在 VM 上执行源码；`input` 作为 `read_*` 的预置标准输入。
pub fn run_vm(source: &str, input: &[u8]) -> Run {
    let mut config = RuntimeConfig::test();
    config.input = input.to_vec();
    match run_source(source, config) {
        Ok(output) => Run::Ok(output),
        Err(error) => Run::Failed(error),
    }
}

/// 编译为 Stardust 并用仓库自带解释器执行；`input` 写入解释器 stdin。
pub fn run_stardust(source: &str, name: &str, input: &[u8]) -> Run {
    let stardust = match build_stardust(source) {
        Ok(text) => text,
        Err(error) => return Run::Failed(format!("build error: {error}")),
    };

    let dir = PathBuf::from("target/differential-tests");
    if let Err(error) = fs::create_dir_all(&dir) {
        return Run::Failed(format!("create {}: {error}", dir.display()));
    }
    let path = dir.join(format!("{name}.sd"));
    if let Err(error) = fs::write(&path, stardust) {
        return Run::Failed(format!("write {}: {error}", path.display()));
    }

    let mut child = match Command::new(INTERPRETER)
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Run::Failed(format!("launch interpreter: {error}")),
    };
    if let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(input)
    {
        return Run::Failed(format!("write interpreter stdin: {error}"));
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => return Run::Failed(format!("wait interpreter: {error}")),
    };

    if output.status.success() {
        Run::Ok(output.stdout)
    } else {
        Run::Failed(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// 断言同一源码在两个后端上的 stdout 逐字节一致。
pub fn assert_same_output(source: &str, name: &str, input: &[u8]) {
    let vm = run_vm(source, input);
    let sd = run_stardust(source, name, input);
    match (&vm, &sd) {
        (Run::Ok(vm_out), Run::Ok(sd_out)) => assert_eq!(
            sd_out,
            vm_out,
            "VM/Stardust stdout mismatch for {name}\nVM: {}\nSD: {}",
            String::from_utf8_lossy(vm_out),
            String::from_utf8_lossy(sd_out)
        ),
        (Run::Failed(vm_err), Run::Failed(_sd_err)) => {
            assert!(
                !vm_err.is_empty(),
                "{name}: VM reported an empty error message"
            );
        }
        (Run::Ok(vm_out), Run::Failed(sd_err)) => panic!(
            "{name}: VM succeeded but Stardust failed\nVM stdout: {}\nSD error: {sd_err}",
            String::from_utf8_lossy(vm_out)
        ),
        (Run::Failed(vm_err), Run::Ok(sd_out)) => panic!(
            "{name}: VM failed but Stardust succeeded\nVM error: {vm_err}\nSD stdout: {}",
            String::from_utf8_lossy(sd_out)
        ),
    }
}

/// 读取与 fixture 同名的 `.in` 文件；不存在时返回空输入。
pub fn fixture_input(path: &Path) -> Vec<u8> {
    fs::read(path.with_extension("in")).unwrap_or_default()
}

/// 对单个 fixture 做 VM/Stardust 差分断言。
pub fn assert_same(name: &str, path: &Path) {
    let source =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let input = fixture_input(path);
    assert_same_output(&source, name, &input);
}

/// 按文件名排序列出目录下的 `.fb` 文件。
pub fn fb_files(dir: &str) -> Vec<PathBuf> {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| panic!("read dir {dir}: {error}"));
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "fb"))
        .collect();
    files.sort();
    files
}
