//! Freebird 源码格式化器（M4-4）。
//!
//! 采用保守策略：只规范化缩进、行尾空白与连续空行，保证
//!
//! - **幂等**：`fmt(fmt(src)) == fmt(src)`；
//! - **不改变语义**：不重排 token、不改写字符串/字符/注释内容；
//! - **保留注释**：`//` 与 `/* ... */` 原样保留（仅调整行首缩进）。
//!
//! 缩进以 4 空格为一个花括号层级；字符串、字符字面量与注释中的 `{`/`}`
//! 不参与层级计算。

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
        out.push_str(content);
        out.push('\n');
        wrote_any = true;

        let delta = scan_line(content, &mut in_block_comment);
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

/// 扫描一行中不属于字符串/字符/注释的 `{` 与 `}`，返回深度增量。
fn scan_line(line: &str, in_block_comment: &mut bool) -> i32 {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut delta = 0i32;

    while i < chars.len() {
        if *in_block_comment {
            if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                *in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        match chars[i] {
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => break,
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                *in_block_comment = true;
                i += 2;
            }
            '"' => {
                i += 1;
                while i < chars.len() {
                    if chars[i] == '\\' {
                        i += 2;
                    } else if chars[i] == '"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            '\'' => {
                if i + 1 < chars.len() && chars[i + 1] == '\\' {
                    // 转义字符字面量：'\n'、'\'' 等。
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                } else if i + 2 < chars.len() && chars[i + 2] == '\'' {
                    // 普通字符字面量：'a'、'{' 等。
                    i += 3;
                } else {
                    // 循环标签 'outer：跳过引号本身。
                    i += 1;
                }
            }
            '{' => {
                delta += 1;
                i += 1;
            }
            '}' => {
                delta -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }

    delta
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
}
