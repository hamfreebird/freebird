//! 依赖解析与 `freebird.lock`（M6-3.3）。
//!
//! 项目模式的依赖解析从 `freebird.toml` 的 `[dependencies]` 出发：
//!
//! - 递归展开路径依赖（依赖自己的 `freebird.toml` 里的依赖同样生效），
//!   被依赖者排在依赖者之前，保证模块注册顺序稳定；
//! - 模块前缀是包名，因此同名包必须指向同一目录，否则报冲突；
//! - 循环依赖在解析阶段报错，并打印环上的包路径；
//! - 每个包的哈希覆盖包内全部 `.fb` 文件（相对路径 + 内容，FNV-1a 64），
//!   用于 `freebird.lock` 的可复现校验。
//!
//! 锁文件由 `freebird lock` 生成，`run`/`check`/`build` 在锁文件存在时校验：
//! 缺包、多包、路径变化或内容哈希不一致都会给出明确错误。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::manifest::Manifest;

/// 锁文件名（位于清单根目录）。
pub const LOCK_FILE: &str = "freebird.lock";

/// 一个已解析的依赖包。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// 包名，同时作为模块前缀（`use name::item;`）。
    pub name: String,
    /// 包根目录（已规范化）。
    pub root: PathBuf,
    /// 入口文件（`freebird.toml` 的 `entry`，或探测到的 `src/lib.fb` 等）。
    pub entry: PathBuf,
    /// 包内全部 `.fb` 文件的内容哈希（16 位十六进制）。
    pub hash: String,
}

/// 锁文件里的一条依赖记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEntry {
    pub name: String,
    /// 相对清单根目录的路径，使用 `/` 分隔。
    pub path: String,
    pub hash: String,
}

/// 递归解析依赖图；返回顺序为“被依赖者在前”。
pub fn resolve(manifest: &Manifest) -> Result<Vec<Package>, String> {
    let mut packages = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<PathBuf> = Vec::new();
    let mut names: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dependency in &manifest.dependencies {
        visit(
            dependency,
            &mut packages,
            &mut visited,
            &mut stack,
            &mut names,
        )?;
    }
    Ok(packages)
}

fn visit(
    dependency: &crate::manifest::Dependency,
    packages: &mut Vec<Package>,
    visited: &mut HashSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
    names: &mut BTreeMap<String, PathBuf>,
) -> Result<(), String> {
    let root = dependency.path.canonicalize().map_err(|error| {
        format!(
            "dependency `{}` at `{}` cannot be resolved: {error}",
            dependency.name,
            dependency.path.display()
        )
    })?;

    match names.get(&dependency.name) {
        Some(existing) if existing != &root => {
            return Err(format!(
                "dependency name `{}` is provided by both `{}` and `{}`; \
                 package names act as module prefixes and must be unique",
                dependency.name,
                existing.display(),
                root.display()
            ));
        }
        _ => {
            names.insert(dependency.name.clone(), root.clone());
        }
    }

    if stack.contains(&root) {
        let chain: Vec<String> = stack
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        return Err(format!(
            "circular dependency detected: {} -> {}",
            chain.join(" -> "),
            root.display()
        ));
    }
    if visited.contains(&root) {
        return Ok(());
    }

    let child_manifest = {
        let manifest_path = root.join("freebird.toml");
        if manifest_path.exists() {
            Some(Manifest::load(&manifest_path)?)
        } else {
            None
        }
    };

    // 先递归依赖的依赖，保证“被依赖者在前”。
    stack.push(root.clone());
    if let Some(child) = &child_manifest {
        for sub in &child.dependencies {
            visit(sub, packages, visited, stack, names)?;
        }
    }
    stack.pop();

    let entry = match &child_manifest {
        Some(child) => {
            let entry = child.root.join(&child.entry);
            if !entry.exists() {
                return Err(format!(
                    "dependency `{}` entry `{}` does not exist",
                    dependency.name,
                    entry.display()
                ));
            }
            entry
        }
        None => pick_entry(&root, &dependency.name)?,
    };

    let hash = package_hash(&root)?;
    packages.push(Package {
        name: dependency.name.clone(),
        root: root.clone(),
        entry,
        hash,
    });
    visited.insert(root);
    Ok(())
}

/// 没有 `freebird.toml` 的依赖目录：按约定探测入口文件。
fn pick_entry(root: &Path, name: &str) -> Result<PathBuf, String> {
    for candidate in ["src/lib.fb", "src/main.fb", "lib.fb", "main.fb"] {
        let path = root.join(candidate);
        if path.exists() {
            return Ok(path);
        }
    }
    Err(format!(
        "dependency `{name}` at `{}` has no entry file (expected src/lib.fb or src/main.fb)",
        root.display()
    ))
}

/// 锁文件路径（清单根目录下的 `freebird.lock`）。
pub fn lock_path(manifest: &Manifest) -> PathBuf {
    manifest.root.join(LOCK_FILE)
}

/// 解析依赖并生成 `freebird.lock`；返回写入的路径。
pub fn write_lock_for(manifest_path: &Path) -> Result<PathBuf, String> {
    let manifest = Manifest::load(manifest_path)?;
    let packages = resolve(&manifest)?;
    write_lock(&manifest, &packages)
}

/// 把已解析的依赖写入 `freebird.lock`。
pub fn write_lock(manifest: &Manifest, packages: &[Package]) -> Result<PathBuf, String> {
    let mut text = String::new();
    text.push_str("# 由 `freebird lock` 生成；请随项目提交以固定依赖内容。\n");
    text.push_str("# 依赖内容变化或调整依赖后，重新运行 `freebird lock`。\n");
    text.push_str("version = 1\n");
    for package in packages {
        text.push_str("\n[[package]]\n");
        text.push_str(&format!("name = \"{}\"\n", package.name));
        text.push_str(&format!(
            "path = \"{}\"\n",
            relative_path(&manifest.root, &package.root)
        ));
        text.push_str(&format!("hash = \"{}\"\n", package.hash));
    }
    let path = lock_path(manifest);
    std::fs::write(&path, text)
        .map_err(|error| format!("failed to write `{}`: {error}", path.display()))?;
    Ok(path)
}

/// 读取锁文件；不存在时返回 `None`。
pub fn read_lock(manifest: &Manifest) -> Result<Option<Vec<LockEntry>>, String> {
    let path = lock_path(manifest);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let entries = parse_lock(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Some(entries))
}

/// 校验锁文件与当前依赖是否一致；没有锁文件时视为通过。
pub fn verify_lock(manifest: &Manifest, packages: &[Package]) -> Result<(), String> {
    let Some(entries) = read_lock(manifest)? else {
        return Ok(());
    };
    let locked: BTreeMap<&str, &LockEntry> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();
    let mut problems: Vec<String> = Vec::new();

    for package in packages {
        let rel = relative_path(&manifest.root, &package.root);
        match locked.get(package.name.as_str()) {
            None => problems.push(format!(
                "dependency `{}` is missing from `{LOCK_FILE}`; run `freebird lock`",
                package.name
            )),
            Some(entry) => {
                if entry.path != rel {
                    problems.push(format!(
                        "dependency `{}` path changed (`{}` -> `{rel}`); run `freebird lock`",
                        package.name, entry.path
                    ));
                }
                if entry.hash != package.hash {
                    problems.push(format!(
                        "dependency `{}` content hash changed (`{}` -> `{}`); \
                         run `freebird lock` after reviewing the change",
                        package.name, entry.hash, package.hash
                    ));
                }
            }
        }
    }
    for name in locked.keys() {
        if !packages.iter().any(|package| package.name == *name) {
            problems.push(format!(
                "`{name}` is locked but no longer a dependency; run `freebird lock`"
            ));
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "`{LOCK_FILE}` is out of date:\n- {}",
            problems.join("\n- ")
        ))
    }
}

fn parse_lock(text: &str) -> Result<Vec<LockEntry>, String> {
    let mut entries: Vec<LockEntry> = Vec::new();
    let mut current: Option<LockEntry> = None;
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[package]]" {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(LockEntry {
                name: String::new(),
                path: String::new(),
                hash: String::new(),
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {line_number}: expected `key = value`"));
        };
        let key = key.trim();
        let value = value.trim();
        if key == "version" {
            if value != "1" {
                return Err(format!(
                    "line {line_number}: unsupported lock version `{value}`"
                ));
            }
            continue;
        }
        let Some(entry) = current.as_mut() else {
            return Err(format!(
                "line {line_number}: `{key}` must appear inside a `[[package]]` section"
            ));
        };
        let value = unquote(value, line_number)?;
        match key {
            "name" => entry.name = value,
            "path" => entry.path = value,
            "hash" => entry.hash = value,
            _ => {}
        }
    }
    if let Some(entry) = current.take() {
        entries.push(entry);
    }
    for entry in &entries {
        if entry.name.is_empty() || entry.path.is_empty() || entry.hash.is_empty() {
            return Err(format!(
                "package entry `{}` needs `name`, `path` and `hash`",
                if entry.name.is_empty() {
                    "<unnamed>"
                } else {
                    &entry.name
                }
            ));
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn unquote(value: &str, line_number: usize) -> Result<String, String> {
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(format!("line {line_number}: expected a quoted string"));
    }
    Ok(value[1..value.len() - 1].to_string())
}

/// 计算包内容哈希：包内全部 `.fb` 文件的相对路径与内容（FNV-1a 64）。
fn package_hash(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_sources(root, &mut files)?;
    files.sort();
    let mut hasher = Fnv1a::new();
    for file in &files {
        let rel = file.strip_prefix(root).unwrap_or(file);
        let rel = rel.to_string_lossy().replace('\\', "/");
        hasher.update(rel.as_bytes());
        hasher.update(&[0]);
        let content = std::fs::read(file)
            .map_err(|error| format!("failed to read `{}`: {error}", file.display()))?;
        hasher.update(&content);
        hasher.update(&[0]);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("failed to read directory `{}`: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("failed to read directory `{}`: {error}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to stat `{}`: {error}", path.display()))?;
        if file_type.is_dir() {
            // 跳过构建产物与版本控制目录，避免哈希随本地状态漂移。
            if name == "target" || name.starts_with('.') {
                continue;
            }
            collect_sources(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "fb") {
            out.push(path);
        }
    }
    Ok(())
}

/// 从 `root` 到 `target` 的相对路径（`/` 分隔）；两者都应是规范路径。
fn relative_path(root: &Path, target: &Path) -> String {
    let root_components: Vec<_> = root.components().collect();
    let target_components: Vec<_> = target.components().collect();
    let mut common = 0;
    while common < root_components.len()
        && common < target_components.len()
        && root_components[common] == target_components[common]
    {
        common += 1;
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..root_components.len() {
        parts.push("..".to_string());
    }
    for component in &target_components[common..] {
        parts.push(component.as_os_str().to_string_lossy().to_string());
    }
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "freebird-deps-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn resolves_transitive_dependencies_in_order() {
        let base = temp_dir("transitive");
        write(
            &base.join("core/src/lib.fb"),
            "pub fn triple(x: i64) -> i64 {\n    x * 3\n}\n",
        );
        write(
            &base.join("core/freebird.toml"),
            "[package]\nname = \"core\"\nentry = \"src/lib.fb\"\n",
        );
        write(
            &base.join("util/src/lib.fb"),
            "use core::triple;\n\npub fn six(x: i64) -> i64 {\n    triple(x) * 2\n}\n",
        );
        write(
            &base.join("util/freebird.toml"),
            "[package]\nname = \"util\"\nentry = \"src/lib.fb\"\n\n[dependencies]\ncore = { path = \"../core\" }\n",
        );
        write(
            &base.join("app/freebird.toml"),
            "[package]\nname = \"app\"\nentry = \"src/main.fb\"\n\n[dependencies]\nutil = { path = \"../util\" }\n",
        );
        write(&base.join("app/src/main.fb"), "fn main() {}\n");

        let manifest = Manifest::load(&base.join("app/freebird.toml")).unwrap();
        let packages = resolve(&manifest).unwrap();
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["core", "util"],
            "dependencies must precede dependents"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn detects_circular_dependencies() {
        let base = temp_dir("cycle");
        write(
            &base.join("a/freebird.toml"),
            "[package]\nname = \"a\"\nentry = \"src/lib.fb\"\n\n[dependencies]\nb = { path = \"../b\" }\n",
        );
        write(&base.join("a/src/lib.fb"), "pub fn fa() {}\n");
        write(
            &base.join("b/freebird.toml"),
            "[package]\nname = \"b\"\nentry = \"src/lib.fb\"\n\n[dependencies]\na = { path = \"../a\" }\n",
        );
        write(&base.join("b/src/lib.fb"), "pub fn fb() {}\n");

        let manifest = Manifest::load(&base.join("a/freebird.toml")).unwrap();
        let error = resolve(&manifest).unwrap_err();
        assert!(
            error.contains("circular dependency"),
            "unexpected error: {error}"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn rejects_duplicate_package_names() {
        let base = temp_dir("dup");
        write(&base.join("core-a/src/lib.fb"), "pub fn f() {}\n");
        write(&base.join("core-b/src/lib.fb"), "pub fn g() {}\n");
        write(&base.join("util/src/lib.fb"), "pub fn h() {}\n");
        write(
            &base.join("util/freebird.toml"),
            "[package]\nname = \"util\"\nentry = \"src/lib.fb\"\n\n[dependencies]\ncore = { path = \"../core-b\" }\n",
        );
        // 同一依赖名 `core` 在两处指向不同目录：模块前缀会冲突。
        write(
            &base.join("app/freebird.toml"),
            "[package]\nname = \"app\"\nentry = \"src/main.fb\"\n\n[dependencies]\nutil = { path = \"../util\" }\ncore = { path = \"../core-a\" }\n",
        );
        write(&base.join("app/src/main.fb"), "fn main() {}\n");

        let manifest = Manifest::load(&base.join("app/freebird.toml")).unwrap();
        let error = resolve(&manifest).unwrap_err();
        assert!(error.contains("unique"), "unexpected error: {error}");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn lock_roundtrip_and_hash_verification() {
        let base = temp_dir("lock");
        write(
            &base.join("util/src/lib.fb"),
            "pub fn double(x: i64) -> i64 {\n    x * 2\n}\n",
        );
        write(
            &base.join("app/freebird.toml"),
            "[package]\nname = \"app\"\nentry = \"src/main.fb\"\n\n[dependencies]\nutil = { path = \"../util\" }\n",
        );
        write(&base.join("app/src/main.fb"), "fn main() {}\n");

        let manifest = Manifest::load(&base.join("app/freebird.toml")).unwrap();
        let packages = resolve(&manifest).unwrap();
        verify_lock(&manifest, &packages).expect("no lock file yet must verify");

        let path = write_lock(&manifest, &packages).unwrap();
        assert!(path.ends_with(LOCK_FILE));
        verify_lock(&manifest, &packages).expect("fresh lock must verify");

        // 依赖内容变化后哈希不再匹配
        write(
            &base.join("util/src/lib.fb"),
            "pub fn double(x: i64) -> i64 {\n    x + x\n}\n",
        );
        let changed = resolve(&manifest).unwrap();
        let error = verify_lock(&manifest, &changed).unwrap_err();
        assert!(error.contains("hash"), "unexpected error: {error}");

        // 重新生成锁文件后恢复
        write_lock(&manifest, &changed).unwrap();
        verify_lock(&manifest, &changed).expect("regenerated lock must verify");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn lock_records_relative_paths() {
        let base = temp_dir("relpath");
        write(&base.join("libs/util/src/lib.fb"), "pub fn f() {}\n");
        write(
            &base.join("app/freebird.toml"),
            "[package]\nname = \"app\"\nentry = \"src/main.fb\"\n\n[dependencies]\nutil = { path = \"../libs/util\" }\n",
        );
        write(&base.join("app/src/main.fb"), "fn main() {}\n");
        let manifest = Manifest::load(&base.join("app/freebird.toml")).unwrap();
        let packages = resolve(&manifest).unwrap();
        let path = write_lock(&manifest, &packages).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("path = \"../libs/util\""),
            "lock text: {text}"
        );
        let entries = read_lock(&manifest).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "util");
        let _ = std::fs::remove_dir_all(base);
    }
}
