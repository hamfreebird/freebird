//! Freebird 源码格式化器（M4-4，M6-3.5 增强）。
//!
//! 规范化分两层：
//!
//! - **行级**：缩进（4 空格/花括号层级）、行尾空白、连续空行折叠；
//!   把 `}` 之后独立成行的 `else` / `else if` 合并回 `} else ...`。
//! - **行内 token 级空白**：字符串、字符、注释之外的多余空白折叠为一个空格；
//!   `,` 前无空格、后接一个空格；`:` 后接一个空格（`::` 保持）；
//!   `(`/`[` 之后与 `)`/`]`、`;` 之前不留空格；`{` 前保留一个空格。
//!
//! 保证：
//!
//! - **幂等**：`fmt(fmt(src)) == fmt(src)`；
//! - **不改变语义**：不重排 token（不做自动折行/拆行）、不改写字符串/字符/注释内容；
//! - **保留注释**：`//` 与 `/* ... */` 原样保留（仅调整行首缩进与前置间距）。

/// 格式化结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatResult {
    pub output: String,
    /// 输出是否与输入不同。
    pub changed: bool,
}

/// 格式化源码。
pub fn format_source(source: &str) -> String {
    format_source_checked(source).output
}

/// 格式化源码并报告是否发生变化。
pub fn format_source_checked(source: &str) -> FormatResult {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    // 安全的换行重排：`}` 之后独立成行的 `else` / `else if` 合并回同一行。
    let normalized = join_brace_else(&normalized);
    let mut depth: usize = 0;
    let mut in_block_comment = false;
    let mut out = String::with_capacity(normalized.len());
    let mut pending_blank = false;
    let mut wrote_any = false;

    for raw_line in normalized.split('\n') {
        let line = raw_line.trim_end();
        let content = line.trim_start();
        if content.is_empty() {
            // 只在已有内容之后保留一个空行；文件开头与末尾的空行会被丢弃。
            pending_blank = wrote_any;
            continue;
        }
        // 保留最多一个空行；`}` 之前不保留空行。
        if pending_blank && !content.starts_with('}') {
            out.push('\n');
        }
        pending_blank = false;

        let leading_closes = content.chars().take_while(|c| *c == '}').count();
        let indent_level = depth.saturating_sub(leading_closes);
        for _ in 0..indent_level {
            out.push_str("    ");
        }
        let (line_out, delta) = normalize_line(content, &mut in_block_comment);
        out.push_str(&line_out);
        out.push('\n');
        wrote_any = true;

        depth = (depth as i32 + delta).max(0) as usize;
    }

    // 去掉文件末尾多余的空行，保留单个换行。
    while out.ends_with("\n\n") {
        out.pop();
    }
    if out.is_empty() {
        return FormatResult {
            output: String::new(),
            changed: !source.is_empty(),
        };
    }

    let changed = out != source;
    FormatResult {
        output: out,
        changed,
    }
}

/// 把 `}` 之后独立成行的 `else` / `else if` 合并回同一行。
fn join_brace_else(source: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in source.split('\n') {
        let trimmed = line.trim();
        let is_else = trimmed == "else"
            || trimmed.starts_with("else ")
            || trimmed.starts_with("else{")
            || trimmed.starts_with("else if");
        if is_else
            && let Some(previous) = lines.last_mut()
            && previous.trim_end().ends_with('}')
        {
            let merged = format!("{} {trimmed}", previous.trim_end());
            *previous = merged;
            continue;
        }
        lines.push(line.to_string());
    }
    lines.join("\n")
}

/// 规范化一行内的空白与标点间距，返回（规范化文本, 花括号深度增量）。
///
/// 字符串、字符与注释内容原样保留；注释只调整前置间距。
fn normalize_line(line: &str, in_block_comment: &mut bool) -> (String, i32) {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut delta = 0i32;
    let mut pending_space = false;
    let mut i = 0;

    while i < chars.len() {
        if *in_block_comment {
            let ch = chars[i];
            out.push(ch);
            i += 1;
            if ch == '*' && i < chars.len() && chars[i] == '/' {
                out.push('/');
                *in_block_comment = false;
                i += 1;
            }
            continue;
        }

        let ch = chars[i];
        match ch {
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => {
                if !out.is_empty() {
                    push_space(&mut out);
                }
                out.extend(chars[i..].iter());
                break;
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                if pending_space {
                    push_space(&mut out);
                }
                pending_space = false;
                out.push_str("/*");
                *in_block_comment = true;
                i += 2;
            }
            '"' | '\'' => {
                if pending_space {
                    push_space(&mut out);
                }
                pending_space = false;
                i = copy_literal(&chars, i, &mut out);
            }
            c if c.is_whitespace() => {
                pending_space = true;
                i += 1;
            }
            ',' => {
                trim_trailing_space(&mut out);
                out.push(',');
                pending_space = true;
                i += 1;
            }
            ':' if i + 1 < chars.len() && chars[i + 1] == ':' => {
                trim_trailing_space(&mut out);
                out.push_str("::");
                pending_space = false;
                i += 2;
            }
            ':' => {
                trim_trailing_space(&mut out);
                out.push(':');
                pending_space = true;
                i += 1;
            }
            '(' | '[' => {
                if pending_space {
                    push_space(&mut out);
                }
                pending_space = false;
                out.push(ch);
                i += 1;
                // 左括号之后不留空格。
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
            }
            ')' | ']' => {
                trim_trailing_space(&mut out);
                out.push(ch);
                pending_space = false;
                i += 1;
            }
            ';' => {
                trim_trailing_space(&mut out);
                out.push(';');
                pending_space = true;
                i += 1;
            }
            '{' => {
                if !out.is_empty()
                    && !out.ends_with(' ')
                    && !out.ends_with('(')
                    && !out.ends_with('{')
                    && !out.ends_with(':')
                {
                    out.push(' ');
                }
                pending_space = false;
                out.push('{');
                delta += 1;
                i += 1;
                // 结构体字面量/枚举载荷：`{` 同行后面还有内容时留一个空格。
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                if i < chars.len() && chars[i] != '}' {
                    out.push(' ');
                }
            }
            '}' => {
                trim_trailing_space(&mut out);
                // 行内 `}`（结构体字面量/单行块）前保留一个空格；行首 `}` 不受影响。
                if !out.is_empty() && !out.ends_with('{') {
                    push_space(&mut out);
                }
                out.push('}');
                delta -= 1;
                pending_space = false;
                i += 1;
            }
            other => {
                if pending_space {
                    push_space(&mut out);
                }
                pending_space = false;
                out.push(other);
                i += 1;
            }
        }
    }

    (out.trim_end().to_string(), delta)
}

/// 原样复制字符串/字符字面量（或循环标签），返回新的索引。
fn copy_literal(chars: &[char], mut i: usize, out: &mut String) -> usize {
    let quote = chars[i];
    out.push(quote);
    i += 1;
    if quote == '\'' {
        if i < chars.len() && chars[i] == '\\' {
            // 转义字符字面量：'\n'、'\'' 等。
            out.push('\\');
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                out.push('\'');
                i += 1;
            }
        } else if i + 1 < chars.len() && chars[i + 1] == '\'' {
            // 普通字符字面量：'a'、'{' 等。
            out.push(chars[i]);
            out.push('\'');
            i += 2;
        }
        // 其余情况是循环标签（'outer），只保留引号本身。
        return i;
    }
    while i < chars.len() {
        let ch = chars[i];
        out.push(ch);
        i += 1;
        if ch == '\\' && i < chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if ch == '"' {
            break;
        }
    }
    i
}

/// 追加一个空格（不产生行首空格或连续空格）。
fn push_space(out: &mut String) {
    if !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
}

/// 删除输出末尾的空格。
fn trim_trailing_space(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parse;

    #[test]
    fn formats_indentation() {
        let source = "fn main() {\nlet x = 1;\nif x > 0 {\nprint(x);\n}\n}\n";
        let expected = "fn main() {\n    let x = 1;\n    if x > 0 {\n        print(x);\n    }\n}\n";
        assert_eq!(format_source(source), expected);
    }

    #[test]
    fn keeps_braces_inside_strings_comments_and_chars() {
        let source = r#"fn main() {
// comment with } and {
let s = "}{";
let c = '{';
/* block } {
   still comment } */
print(s);
print_char(c);
}
"#;
        let formatted = format_source(source);
        assert!(formatted.contains("// comment with } and {"));
        assert!(formatted.contains("let s = \"}{\";"));
        assert!(formatted.contains("let c = '{';"));
        assert!(formatted.contains("/* block } {"));
        // 注释/字符串中的括号不改变层级：print 仍在函数体内（4 空格）。
        assert!(formatted.contains("\n    print(s);"));
    }

    #[test]
    fn collapses_blank_lines_and_trailing_whitespace() {
        let source = "fn main() {   \n\n\n    print(1);   \n\n}\n\n\n";
        let formatted = format_source(source);
        assert_eq!(formatted, "fn main() {\n\n    print(1);\n}\n");
    }

    #[test]
    fn is_idempotent_and_parseable() {
        let source = "fn main(){let mut i=0;while i<3{if i==1{print(i);}i=i+1;}}\n";
        let once = format_source(source);
        let twice = format_source(&once);
        assert_eq!(once, twice);
        let (_, errors) = parse(&once);
        assert!(errors.is_empty(), "formatted output must parse: {errors:?}");
    }

    #[test]
    fn normalizes_crlf() {
        let source = "fn main() {\r\nprint(1);\r\n}\r\n";
        let formatted = format_source(source);
        assert_eq!(formatted, "fn main() {\n    print(1);\n}\n");
    }

    #[test]
    fn normalizes_inline_spacing() {
        let source = "fn main( ) {\nlet a=[1,2 , 3] ;\nprint( a[0] ) ;\nlet p=Point{x:1,y:2};\n}\n";
        let formatted = format_source(source);
        assert!(formatted.contains("fn main() {"), "{formatted}");
        assert!(formatted.contains("let a=[1, 2, 3];"), "{formatted}");
        assert!(formatted.contains("print(a[0]);"), "{formatted}");
        assert!(formatted.contains("Point { x: 1, y: 2 }"), "{formatted}");
    }

    #[test]
    fn keeps_path_separators_and_strings() {
        let source = "use a::b;\nfn main() {\nlet s = \"a , b : c\";\nprint_str(s);\n}\n";
        let formatted = format_source(source);
        assert!(formatted.contains("use a::b;"), "{formatted}");
        assert!(formatted.contains("\"a , b : c\""), "{formatted}");
    }

    #[test]
    fn joins_brace_and_else() {
        let source = "fn main() {\nif 1 > 0 {\nprint(1);\n}\nelse {\nprint(2);\n}\n}\n";
        let formatted = format_source(source);
        assert!(formatted.contains("} else {"), "{formatted}");
        assert!(!formatted.contains("\nelse"), "{formatted}");
    }

    #[test]
    fn formats_examples_idempotently_and_parseably() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("examples directory") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|ext| ext != "fb") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read example");
            let once = format_source(&source);
            assert_eq!(
                once,
                format_source(&once),
                "{} is not idempotent",
                path.display()
            );
            let (_, errors) = parse(&once);
            assert!(
                errors.is_empty(),
                "{} must parse after formatting: {errors:?}",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 0, "examples/ should contain .fb files");
    }
}
