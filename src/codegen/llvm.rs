//! LLVM 后端：复用 Stardust 的 LLVM 编译器。
//!
//! Freebird 的 MIR 已经能完整降级为 Stardust 源码（见 [`crate::codegen::stardust`]），
//! 而 Stardust 自带成熟的 LLVM IR 编译器。因此 LLVM 后端的职责是编排：
//!
//! ```text
//! Freebird 源码 → MIR → Stardust 源码 → stardust --compile → LLVM IR
//!                                      └→ clang -O<level> IR → 可执行文件
//! ```
//!
//! 这样两个后端共享同一套值布局与堆块语义，避免出现“VM 能跑、LLVM 语义不同”的
//! 第二套实现。Stardust 工具链的定位见 `readme.md` §10、§13。
//!
//! 需要系统安装 Stardust（0.1.6+）与 clang：
//! - Stardust 可执行文件按 `--stardust-bin` → `STARDUST_BIN` → `PATH` →
//!   兄弟仓库 `../esolang_stardust/target/{release,debug}/` 的顺序查找；
//! - clang 按 `CLANG` → `CC` → `PATH` 查找。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::pipeline::{CompiledProgram, compile_program};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// LLVM 后端的产物类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlvmEmit {
    /// 生成可读的 LLVM IR 文本。
    Ir,
    /// 生成可执行文件（调用 clang）。
    Executable,
}

/// LLVM 构建选项。
#[derive(Debug, Clone)]
pub struct LlvmOptions {
    pub emit: LlvmEmit,
    /// 可执行文件的输出路径（`LlvmEmit::Executable` 时必需）。
    pub output: Option<PathBuf>,
    /// 显式指定的 Stardust 可执行文件。
    pub stardust_bin: Option<PathBuf>,
    /// 传给 clang 的优化级别（0–3）。
    pub opt_level: u8,
}

impl Default for LlvmOptions {
    fn default() -> Self {
        Self {
            emit: LlvmEmit::Ir,
            output: None,
            stardust_bin: None,
            opt_level: 0,
        }
    }
}

impl LlvmOptions {
    pub fn ir() -> Self {
        Self::default()
    }

    pub fn executable(output: PathBuf) -> Self {
        Self {
            emit: LlvmEmit::Executable,
            output: Some(output),
            ..Self::default()
        }
    }
}

/// LLVM 构建产物。
#[derive(Debug, Clone)]
pub enum LlvmArtifact {
    /// 生成的 LLVM IR 文本。
    Ir(String),
    /// 生成的可执行文件路径。
    Executable(PathBuf),
}

/// 编译源码到 LLVM IR 或可执行文件。
pub fn compile_llvm(source: &str, options: &LlvmOptions) -> Result<LlvmArtifact, String> {
    let program = compile_program(source)?;
    compile_llvm_compiled(&program, options)
}

/// 从已编译的 MIR/Stardust 产物构建 LLVM IR 或可执行文件（项目模式复用）。
pub fn compile_llvm_compiled(
    program: &CompiledProgram,
    options: &LlvmOptions,
) -> Result<LlvmArtifact, String> {
    // LLVM 后端只传递 char_mode；运行时限制注解不会被 LLVM 程序执行。
    // 明确拒绝，避免用户误以为限制生效。
    if let Some(annotation) = program.stardust.lines().find(|line| {
        let line = line.trim_start();
        line.starts_with("#[max_") || line.starts_with("#[time_limit")
    }) {
        return Err(format!(
            "`--target llvm` does not enforce runtime-limit attributes (found `{annotation}`); \
             use the VM or `--target stardust`, or remove the attribute"
        ));
    }
    let stardust = locate_stardust(options.stardust_bin.as_deref())?;
    let temp = TempFiles::new()?;
    std::fs::write(&temp.stardust, &program.stardust)
        .map_err(|e| format!("failed to write temporary Stardust source: {e}"))?;

    run_stardust_compile(&stardust, &temp.stardust, &temp.ir)?;

    let ir = std::fs::read_to_string(&temp.ir)
        .map_err(|e| format!("failed to read LLVM IR produced by Stardust: {e}"))?;

    match options.emit {
        LlvmEmit::Ir => Ok(LlvmArtifact::Ir(ir)),
        LlvmEmit::Executable => {
            let output = options
                .output
                .clone()
                .ok_or_else(|| "executable output path is required".to_string())?;
            let clang = locate_clang()?;
            run_clang(&clang, &temp.ir, &output, options.opt_level)?;
            Ok(LlvmArtifact::Executable(output))
        }
    }
}

/// 便利函数：只生成 LLVM IR 文本。
pub fn compile_llvm_ir(source: &str) -> Result<String, String> {
    match compile_llvm(source, &LlvmOptions::ir())? {
        LlvmArtifact::Ir(ir) => Ok(ir),
        LlvmArtifact::Executable(_) => unreachable!("ir options produce ir"),
    }
}

/// 临时文件集合，Drop 时清理。
struct TempFiles {
    stardust: PathBuf,
    ir: PathBuf,
}

impl TempFiles {
    fn new() -> Result<Self, String> {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = format!("freebird-{}-{id}", std::process::id());
        let dir = std::env::temp_dir();
        Ok(Self {
            stardust: dir.join(format!("{base}.sd")),
            ir: dir.join(format!("{base}.ll")),
        })
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.stardust);
        let _ = std::fs::remove_file(&self.ir);
    }
}

fn run_stardust_compile(stardust: &Path, source: &Path, output: &Path) -> Result<(), String> {
    let result = Command::new(stardust)
        .arg("--compile")
        .arg(source)
        .arg(output)
        .output()
        .map_err(|e| {
            format!(
                "failed to run Stardust compiler `{}`: {e}",
                stardust.display()
            )
        })?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        return Err(format!(
            "Stardust LLVM compilation failed:\n{}{}",
            stdout.trim_end(),
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", stderr.trim_end())
            }
        ));
    }
    Ok(())
}

fn run_clang(clang: &Path, ir: &Path, output: &Path, opt_level: u8) -> Result<(), String> {
    let result = Command::new(clang)
        .arg(format!("-O{opt_level}"))
        .arg(ir)
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|e| format!("failed to run clang `{}`: {e}", clang.display()))?;
    if !result.status.success() {
        return Err(format!(
            "clang failed to link LLVM IR:\n{}",
            String::from_utf8_lossy(&result.stderr).trim_end()
        ));
    }
    Ok(())
}

/// 查找 Stardust 可执行文件。
pub fn locate_stardust(explicit: Option<&Path>) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = explicit {
        candidates.push(path.to_path_buf());
    }
    if let Some(path) = std::env::var_os("STARDUST_BIN") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(path) = find_in_path("stardust") {
        candidates.push(path);
    }
    candidates.extend(bundled_stardust_candidates());
    for base in sibling_bases() {
        candidates.push(base.join("target/release/esolang_stardust"));
        candidates.push(base.join("target/debug/esolang_stardust"));
    }

    for candidate in candidates {
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }

    Err(
        "Stardust toolchain not found. Install Stardust 0.1.6+ and put `stardust` on PATH, \
         set `STARDUST_BIN`, pass `--stardust-bin <path>`, or build the sibling repository \
         `../esolang_stardust` (release/debug)."
            .to_string(),
    )
}

/// 查找 clang。
pub fn locate_clang() -> Result<PathBuf, String> {
    for var in ["CLANG", "CC"] {
        if let Some(path) = std::env::var_os(var) {
            let path = PathBuf::from(path);
            if is_executable(&path) {
                return Ok(path);
            }
        }
    }
    if let Some(path) = find_in_path("clang") {
        return Ok(path);
    }
    Err("clang not found; install LLVM/clang or set `CLANG`".to_string())
}

fn sibling_bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(parent) = cwd.parent() {
            bases.push(parent.join("esolang_stardust"));
        }
        bases.push(cwd.join("esolang_stardust"));
    }
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR")
        && let Some(parent) = Path::new(manifest).parent()
    {
        bases.push(parent.join("esolang_stardust"));
    }
    bases
}

/// 仓库自带的 Stardust 可执行文件（`stardust/stardust-linux-x86_64`）。
///
/// CI 与普通用户无需单独安装 Stardust；找不到时再回退到 PATH / 兄弟仓库。
fn bundled_stardust_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("stardust/stardust-linux-x86_64"));
    }
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(Path::new(manifest).join("stardust/stardust-linux-x86_64"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("stardust/stardust-linux-x86_64"));
        candidates.push(dir.join("../../stardust/stardust-linux-x86_64"));
        candidates.push(dir.join("../../../stardust/stardust-linux-x86_64"));
    }
    candidates
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llvm_ir_contains_module_for_hello() {
        if locate_stardust(None).is_err() {
            eprintln!("skipping: Stardust toolchain not available");
            return;
        }
        let ir = compile_llvm_ir("fn main() { print(42); }").expect("llvm ir");
        assert!(ir.contains("define"), "unexpected IR: {ir}");
        assert!(ir.contains("main"));
    }

    #[test]
    fn llvm_supports_file_io() {
        // M6-2.1：Stardust LLVM 后端实现 FileRead/FileWrite 后，Freebird 不再编译期拒绝。
        if locate_stardust(None).is_err() {
            eprintln!("skipping: Stardust toolchain not available");
            return;
        }
        let ir = compile_llvm_ir(r#"fn main() { let s = file_read("x.txt"); print_str(s); }"#)
            .expect("file I/O should compile on the LLVM backend");
        assert!(
            ir.contains("@sd_file_read") || ir.contains("@fread"),
            "LLVM IR should contain the file I/O runtime: {ir}"
        );
    }
}
