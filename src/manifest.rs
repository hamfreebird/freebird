//! `freebird.toml` 最小项目清单（M4-5）。
//!
//! 支持的语法（刻意保持最小）：
//!
//! ```toml
//! [package]
//! name = "app"
//! entry = "src/main.fb"
//!
//! [dependencies]
//! util = { path = "../util" }
//! other = "../other"
//! ```
//!
//! 只支持路径依赖（无版本解析/registry）。依赖目录若自带 `freebird.toml`，
//! 使用其 `entry`；否则依次尝试 `src/lib.fb`、`src/main.fb`、`lib.fb`、`main.fb`。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub entry: String,
    /// 清单文件所在目录（依赖路径相对于它解析）。
    pub root: PathBuf,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub path: PathBuf,
}

impl Manifest {
    /// 读取清单。`path` 可以是 `freebird.toml` 或包含它的目录。
    pub fn load(path: &Path) -> Result<Self, String> {
        let manifest_path = if path.is_dir() {
            path.join("freebird.toml")
        } else {
            path.to_path_buf()
        };
        let text = std::fs::read_to_string(&manifest_path).map_err(|error| {
            format!(
                "failed to read manifest `{}`: {error}",
                manifest_path.display()
            )
        })?;
        let root = manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let root = root.canonicalize().unwrap_or(root);
        parse(&text, root).map_err(|error| format!("{}: {error}", manifest_path.display()))
    }
}

impl Dependency {
    /// 解析依赖入口文件。
    pub fn entry_path(&self) -> Result<PathBuf, String> {
        let manifest_path = self.path.join("freebird.toml");
        if manifest_path.exists() {
            let manifest = Manifest::load(&manifest_path)?;
            return Ok(manifest.root.join(&manifest.entry));
        }
        for candidate in ["src/lib.fb", "src/main.fb", "lib.fb", "main.fb"] {
            let path = self.path.join(candidate);
            if path.exists() {
                return Ok(path);
            }
        }
        Err(format!(
            "dependency `{}` at `{}` has no entry file (expected src/lib.fb or src/main.fb)",
            self.name,
            self.path.display()
        ))
    }
}

fn parse(text: &str, root: PathBuf) -> Result<Manifest, String> {
    let mut name: Option<String> = None;
    let mut entry: Option<String> = None;
    let mut dependencies = Vec::new();
    let mut section = String::new();

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {line_number}: expected `key = value`"));
        };
        let key = key.trim();
        let value = value.trim();
        match section.as_str() {
            "package" => match key {
                "name" => name = Some(parse_string(value, line_number)?),
                "entry" => entry = Some(parse_string(value, line_number)?),
                _ => {}
            },
            "dependencies" => {
                let path = parse_dependency_path(value, line_number)?;
                dependencies.push(Dependency {
                    name: key.to_string(),
                    path: root.join(path),
                });
            }
            _ => {}
        }
    }

    let default_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("app")
        .to_string();
    Ok(Manifest {
        name: name.unwrap_or(default_name),
        entry: entry.unwrap_or_else(|| "src/main.fb".to_string()),
        root,
        dependencies,
    })
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
        } else if ch == '#' {
            return &line[..index];
        }
    }
    line
}

fn parse_string(value: &str, line_number: usize) -> Result<String, String> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(format!("line {line_number}: expected a quoted string"));
    }
    let inner = &value[1..value.len() - 1];
    let mut output = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('"') => output.push('"'),
                Some('\\') => output.push('\\'),
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }
    Ok(output)
}

fn parse_dependency_path(value: &str, line_number: usize) -> Result<String, String> {
    let value = value.trim();
    if value.starts_with('"') {
        return parse_string(value, line_number);
    }
    if value.starts_with('{') && value.ends_with('}') {
        let inner = &value[1..value.len() - 1];
        for part in inner.split(',') {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            if key.trim() == "path" {
                return parse_string(value.trim(), line_number);
            }
        }
        return Err(format!(
            "line {line_number}: dependency table is missing `path`"
        ));
    }
    Err(format!(
        "line {line_number}: dependency must be a path string or `{{ path = \"...\" }}`"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(text: &str) -> Manifest {
        parse(text, PathBuf::from("/tmp/app")).unwrap()
    }

    #[test]
    fn parses_package_and_inline_dependencies() {
        let manifest = parse_str(
            r#"
# project manifest
[package]
name = "demo"
entry = "src/main.fb"   # entry point

[dependencies]
util = { path = "../util" }
other = "../other"
"#,
        );
        assert_eq!(manifest.name, "demo");
        assert_eq!(manifest.entry, "src/main.fb");
        assert_eq!(manifest.dependencies.len(), 2);
        assert_eq!(manifest.dependencies[0].name, "util");
        assert_eq!(
            manifest.dependencies[0].path,
            PathBuf::from("/tmp/app/../util")
        );
        assert_eq!(
            manifest.dependencies[1].path,
            PathBuf::from("/tmp/app/../other")
        );
    }

    #[test]
    fn defaults_name_and_entry() {
        let manifest = parse_str("[package]\n");
        assert_eq!(manifest.name, "app");
        assert_eq!(manifest.entry, "src/main.fb");
    }

    #[test]
    fn rejects_dependency_without_path() {
        let error = parse(
            "[dependencies]\nutil = { version = \"1\" }\n",
            PathBuf::from("/tmp"),
        )
        .unwrap_err();
        assert!(
            error.contains("missing `path`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn hashes_inside_strings_are_preserved() {
        let manifest = parse_str("[package]\nname = \"a#b\"\n");
        assert_eq!(manifest.name, "a#b");
    }
}
