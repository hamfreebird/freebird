//! Freebird REPL — 交互式读取-求值-打印循环
//!
//! 支持多行输入（括号未闭合时继续读取）、历史（`:history` / `:run n`，
//! 持久化到状态目录，可跨会话）、会话内 `let` 绑定（后续输入可用）、
//! `:load <file>`、`:type <expr>` 以及 `:stack`/`:vars`/`:info`/`:clear`。
//!
//! 行编辑（方向键/补全）不在实现范围内：REPL 只用 `std`，保持离线可构建；
//! 需要行编辑时可用 `rlwrap freebird repl` 之类的外部包装。

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::borrowck::check_borrow;
use crate::frontend::parse;
use crate::hir::lower;
use crate::mir::MirGen;
use crate::typeck::{TypeTable, check_program_with_warnings};
use crate::vm::Vm;
use crate::vm::config::RuntimeConfig;

/// 多行历史条目内部的行分隔符（写入文件时用记录分隔符 UF）。
const HISTORY_LINE_SEPARATOR: char = '\u{1f}';

/// 持久历史保留的最大条数（加载时只取最近这么多条）。
const HISTORY_LIMIT: usize = 200;

/// REPL 状态
pub struct Repl {
    vm: Vm,
    line_count: usize,
    /// 累积的函数/类型声明
    accumulated_decls: String,
    /// 会话内累积的 `let` 绑定（后续输入可用；每次执行重放）
    session_bindings: Vec<String>,
    /// 已执行的完整输入（用于 :history / :run）
    history: Vec<String>,
    /// 多行输入缓冲
    buffer: String,
    /// 持久历史文件路径（`None` 表示不持久化）
    history_path: Option<PathBuf>,
}

impl Repl {
    pub fn new() -> Self {
        let history_path = history_file_path();
        let history = history_path
            .as_deref()
            .map(load_history)
            .unwrap_or_default();
        Self {
            vm: Vm::with_config(RuntimeConfig::test()),
            line_count: 0,
            accumulated_decls: String::new(),
            session_bindings: Vec::new(),
            history,
            buffer: String::new(),
            history_path,
        }
    }

    /// 用指定历史文件创建 REPL（测试用；`None` 表示禁用持久化）。
    pub fn with_history_file(path: Option<PathBuf>) -> Self {
        let history = path.as_deref().map(load_history).unwrap_or_default();
        Self {
            vm: Vm::with_config(RuntimeConfig::test()),
            line_count: 0,
            accumulated_decls: String::new(),
            session_bindings: Vec::new(),
            history,
            buffer: String::new(),
            history_path: path,
        }
    }

    /// 启动 REPL 主循环
    pub fn run(&mut self) -> io::Result<()> {
        println!("Freebird REPL v0.1.0");
        println!("Type :help for commands, :q to quit.\n");

        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut input = String::new();

        loop {
            self.line_count += 1;
            let prompt = if self.buffer.is_empty() {
                "fb> "
            } else {
                "... "
            };
            print!("{prompt}");
            io::stdout().flush()?;

            input.clear();
            if reader.read_line(&mut input)? == 0 {
                if !self.buffer.trim().is_empty() {
                    let code = std::mem::take(&mut self.buffer);
                    self.remember(&code);
                    self.execute_input(&code);
                }
                break;
            }

            let line = input.trim_end();
            if self.buffer.is_empty() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.starts_with(':') {
                    if self.handle_command(trimmed) {
                        break;
                    }
                    continue;
                }
            }

            if !self.buffer.is_empty() {
                self.buffer.push('\n');
            }
            self.buffer.push_str(line);

            if needs_more_input(&self.buffer) {
                continue;
            }

            let code = std::mem::take(&mut self.buffer);
            self.remember(&code);
            self.execute_input(&code);
        }

        println!("\nBye!");
        Ok(())
    }

    /// 处理 `:` 命令；返回 true 表示退出。
    fn handle_command(&mut self, line: &str) -> bool {
        let mut parts = line.splitn(2, char::is_whitespace);
        let command = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();

        match command {
            ":q" | ":quit" => return true,
            ":help" => self.cmd_help(),
            ":stack" => self.cmd_stack(),
            ":vars" => self.cmd_vars(),
            ":info" => self.cmd_info(),
            ":clear" => {
                self.accumulated_decls.clear();
                self.session_bindings.clear();
                println!("Cleared declarations and session bindings.");
            }
            ":history" => self.cmd_history(),
            ":run" => self.cmd_run(arg),
            ":load" => self.cmd_load(arg),
            ":type" => self.cmd_type(arg),
            other => println!("Unknown command: {other}. Type :help for commands."),
        }
        false
    }

    /// 记录历史（连续重复只保留一条），并追加写入持久历史文件。
    fn remember(&mut self, code: &str) {
        let item = code.trim().to_string();
        if item.is_empty() {
            return;
        }
        if self.history.last() == Some(&item) {
            return;
        }
        self.history.push(item.clone());
        if let Some(path) = &self.history_path
            && let Err(error) = append_history(path, &item)
        {
            eprintln!("  (history not saved: {error})");
        }
    }

    fn execute_input(&mut self, code: &str) {
        let trimmed = code.trim();
        if trimmed.is_empty() {
            return;
        }
        if is_declaration_block(trimmed) {
            self.add_declarations(code);
        } else if is_binding_statement(trimmed) {
            self.add_binding(code);
        } else {
            self.execute_expression(code);
        }
    }

    /// 验证并累积顶层声明（解析/名称解析失败时不污染 REPL 状态）。
    fn add_declarations(&mut self, code: &str) {
        let candidate = if self.accumulated_decls.is_empty() {
            format!("{code}\n")
        } else {
            format!("{}\n{code}\n", self.accumulated_decls)
        };
        let (ast, parse_errors) = parse(&candidate);
        if !parse_errors.is_empty() {
            for error in &parse_errors {
                eprintln!("  parse error: {error}");
            }
            return;
        }
        let (_, resolve_errors) = lower(&ast);
        if !resolve_errors.is_empty() {
            for error in &resolve_errors {
                eprintln!("  resolve error: {error}");
            }
            return;
        }
        self.accumulated_decls = candidate;
        println!("  [declared]");
    }

    /// 把输入包装进 `_repl_main` 执行。
    fn execute_expression(&mut self, code: &str) {
        self.run_session(code);
    }

    /// 构造会话源码：累积声明 + `_repl_main`（先重放会话 `let` 绑定，再执行 `tail`）。
    ///
    /// 重放意味着之前 `let` 的初始化表达式会再次执行：纯计算没有影响，
    /// 但含 I/O 或随机性的初始化会重复产生副作用。
    fn session_source(&self, tail: &str) -> String {
        let mut source = String::new();
        if !self.accumulated_decls.is_empty() {
            source.push_str(&self.accumulated_decls);
            source.push('\n');
        }
        source.push_str("fn _repl_main() {\n");
        for binding in &self.session_bindings {
            source.push_str(binding);
            source.push('\n');
        }
        if !tail.trim().is_empty() {
            source.push_str(tail);
            source.push('\n');
        }
        source.push_str("}\n");
        source
    }

    /// 记录 `let` 绑定：整段会话编译失败时回滚，不污染会话状态。
    fn add_binding(&mut self, code: &str) {
        self.session_bindings.push(code.trim().to_string());
        if !self.run_session("") {
            self.session_bindings.pop();
        }
    }

    /// 编译并执行会话源码；返回是否成功。
    fn run_session(&mut self, tail: &str) -> bool {
        let source = self.session_source(tail);

        let (ast, ast_errors) = parse(&source);
        if !ast_errors.is_empty() {
            for error in &ast_errors {
                eprintln!("  parse error: {error}");
            }
            return false;
        }

        let (hir, hir_errors) = lower(&ast);
        if !hir_errors.is_empty() {
            for error in &hir_errors {
                eprintln!("  resolve error: {error}");
            }
            return false;
        }

        let type_table = TypeTable::from_ast_decls(&ast.decls);
        let (type_errors, warnings) = check_program_with_warnings(&hir, &type_table);
        for warning in &warnings {
            println!("  warning: {warning}");
        }
        if !type_errors.is_empty() {
            for error in &type_errors {
                eprintln!("  type error: {error}");
            }
            return false;
        }
        let borrow_errors = check_borrow(&hir);
        if !borrow_errors.is_empty() {
            for error in &borrow_errors {
                eprintln!("  borrow error: {error}");
            }
            return false;
        }
        let mut generator = MirGen::new().with_type_table(type_table);
        let functions = generator.gen_program(&hir);

        // 找到 _repl_main 函数
        let main_mir = functions
            .iter()
            .find(|(_, mir)| !mir.is_empty())
            .map(|(_, mir)| mir.clone());

        let Some(mir) = main_mir else {
            eprintln!("  no executable code generated");
            return false;
        };
        // 每次输入都重放整段会话源码，旧的调用帧不能复用。
        self.vm = Vm::with_config(RuntimeConfig::test());
        for (fn_id, f_mir) in &functions {
            self.vm.register_fn(*fn_id, f_mir.clone());
        }

        let result = self.vm.execute_mir(mir);
        let output = self.vm.take_output();
        if !output.is_empty() {
            print!("{}", String::from_utf8_lossy(&output));
            let _ = io::stdout().flush();
        }

        match result {
            Ok(()) => {
                if let Some(frame) = self.vm.call_stack.last()
                    && let Some(val) = frame.data.top_value()
                {
                    println!("  => {val}");
                }
                true
            }
            Err(e) => {
                eprintln!("  runtime error: {e}");
                false
            }
        }
    }

    fn cmd_history(&self) {
        if self.history.is_empty() {
            println!("  (empty)");
            return;
        }
        for (index, entry) in self.history.iter().enumerate() {
            let first_line = entry.lines().next().unwrap_or("");
            println!("  {:4}  {first_line}", index + 1);
        }
    }

    fn cmd_run(&mut self, arg: &str) {
        let Ok(index) = arg.parse::<usize>() else {
            println!("Usage: :run <n> (see :history)");
            return;
        };
        let Some(code) = self.history.get(index.saturating_sub(1)).cloned() else {
            println!("No history entry {index}");
            return;
        };
        println!("  [replaying {index}]");
        self.execute_input(&code);
    }

    fn cmd_load(&mut self, path: &str) {
        if path.is_empty() {
            println!("Usage: :load <file.fb>");
            return;
        }
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("  error reading {path}: {error}");
                return;
            }
        };
        if source.contains("fn main") {
            match crate::run_source(&source, RuntimeConfig::test()) {
                Ok(output) => print!("{}", String::from_utf8_lossy(&output)),
                Err(error) => eprintln!("  error: {error}"),
            }
        } else {
            self.add_declarations(&source);
        }
    }

    fn cmd_type(&mut self, expr: &str) {
        if expr.is_empty() {
            println!("Usage: :type <expression>");
            return;
        }
        match crate::typeck::infer_expression_type(&self.accumulated_decls, expr) {
            Ok(ty) => println!("  => {ty}"),
            Err(errors) => {
                for error in errors {
                    eprintln!("  type error: {error}");
                }
            }
        }
    }

    fn cmd_help(&self) {
        println!("Commands:");
        println!("  :stack        — display current stack contents");
        println!("  :vars         — display session bindings and variable bindings");
        println!("  :info         — display VM state info");
        println!("  :type <expr>  — infer the type of an expression");
        println!("  :load <file>  — load declarations (or execute a file with `fn main`)");
        println!("  :history      — list executed inputs (persisted across sessions)");
        println!("  :run <n>      — replay history entry n");
        println!("  :clear        — clear accumulated declarations and session bindings");
        println!("  :q/:quit      — exit REPL");
        println!("  :help         — show this help");
        println!();
        println!("`let` bindings persist across inputs within a session (replayed on each input);");
        println!("line editing is not built in — use `rlwrap freebird repl` if needed.");
    }

    fn cmd_stack(&self) {
        match self.vm.call_stack.last() {
            Some(frame) => {
                println!("Stack (bottom → top):");
                for (i, (id, val)) in frame.data.iter_bottom_to_top().enumerate() {
                    println!("  [{i}] {id:?}: {val}");
                }
                if frame.data.is_empty() {
                    println!("  (empty)");
                }
            }
            None => println!("  (no active frame)"),
        }
    }

    fn cmd_vars(&self) {
        if !self.session_bindings.is_empty() {
            println!("Session bindings (replayed each input):");
            for binding in &self.session_bindings {
                println!("  {}", binding.trim());
            }
        }
        match self.vm.call_stack.last() {
            Some(frame) => {
                let bindings = frame.vars.current_frame_bindings();
                if bindings.is_empty() {
                    if self.session_bindings.is_empty() {
                        println!("  (no variables)");
                    }
                } else {
                    println!("Variables:");
                    for (name, id) in &bindings {
                        println!("  {name} → {id:?}");
                    }
                }
            }
            None => {
                if self.session_bindings.is_empty() {
                    println!("  (no active frame)");
                }
            }
        }
    }

    fn cmd_info(&self) {
        println!("VM State:");
        println!("  step_count: {}", self.vm.step_count);
        println!("  call_depth: {}", self.vm.call_stack.len());
        println!("  remote_stacks: {}", self.vm.registry.remote_stacks.len());
        match self.vm.call_stack.last() {
            Some(frame) => {
                println!("  frame_id: {:?}", frame.stack_id);
                println!("  pc: {} / {}", frame.pc, frame.instructions.len());
                println!("  data_stack_len: {}", frame.data.len());
            }
            None => println!("  (no active frame)"),
        }
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

/// `let` / `let mut` 绑定：会话内保留，后续输入可用（每次输入重放）。
fn is_binding_statement(code: &str) -> bool {
    code.trim_start().starts_with("let ")
}

/// 持久历史文件路径：
/// `FREEBIRD_REPL_HISTORY` → `$XDG_STATE_HOME/freebird/repl_history`
/// → `~/.local/state/freebird/repl_history`。
fn history_file_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("FREEBIRD_REPL_HISTORY")
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    if let Ok(state) = std::env::var("XDG_STATE_HOME")
        && !state.is_empty()
    {
        return Some(PathBuf::from(state).join("freebird").join("repl_history"));
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("freebird")
            .join("repl_history")
    })
}

/// 读取持久历史（只保留最近 [`HISTORY_LIMIT`] 条）。
///
/// 每条记录占一行，多行输入内部的换行以 [`HISTORY_LINE_SEPARATOR`] 存储。
fn load_history(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut history: Vec<String> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.replace(HISTORY_LINE_SEPARATOR, "\n"))
        .collect();
    let excess = history.len().saturating_sub(HISTORY_LIMIT);
    if excess > 0 {
        history.drain(..excess);
    }
    history
}

/// 追加一条历史记录（多行输入压成一行）。
fn append_history(path: &Path, item: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let record = item.replace('\n', &HISTORY_LINE_SEPARATOR.to_string());
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{record}")
}

/// 判断一行是否开启顶层声明（可累积）。
fn is_declaration_block(code: &str) -> bool {
    let first = code.trim_start();
    [
        "fn ",
        "pub fn ",
        "struct ",
        "pub struct ",
        "enum ",
        "pub enum ",
        "extern ",
        "use ",
        "impl ",
        "mod ",
        "pub mod ",
    ]
    .iter()
    .any(|prefix| first.starts_with(prefix))
}

/// 括号/字符串/块注释未闭合时返回 true，REPL 继续读取下一行。
fn needs_more_input(input: &str) -> bool {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut parens = 0i32;
    let mut brackets = 0i32;
    let mut braces = 0i32;
    let mut in_block_comment = false;

    while i < chars.len() {
        if in_block_comment {
            if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        match chars[i] {
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                in_block_comment = true;
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
                if i > chars.len() {
                    return true; // 未闭合字符串
                }
            }
            '\'' => {
                if i + 1 < chars.len() && chars[i + 1] == '\\' {
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                } else if i + 2 < chars.len() && chars[i + 2] == '\'' {
                    i += 3;
                } else {
                    // 循环标签 'outer
                    i += 1;
                }
            }
            '(' => {
                parens += 1;
                i += 1;
            }
            ')' => {
                parens -= 1;
                i += 1;
            }
            '[' => {
                brackets += 1;
                i += 1;
            }
            ']' => {
                brackets -= 1;
                i += 1;
            }
            '{' => {
                braces += 1;
                i += 1;
            }
            '}' => {
                braces -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }

    parens > 0 || brackets > 0 || braces > 0 || in_block_comment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_detection_tracks_brackets_and_strings() {
        assert!(needs_more_input("fn main() {"));
        assert!(!needs_more_input("fn main() { print(1); }"));
        assert!(needs_more_input("let x = foo("));
        assert!(!needs_more_input("let x = foo(1);"));
        assert!(!needs_more_input("let s = \"}\";"));
        assert!(!needs_more_input("let c = '{';"));
        assert!(needs_more_input("/* unclosed"));
        assert!(!needs_more_input("/* closed */ print(1);"));
        assert!(!needs_more_input("'outer: loop { break 'outer; }"));
    }

    #[test]
    fn declaration_detection() {
        assert!(is_declaration_block("fn add(a: i64) -> i64 { a }"));
        assert!(is_declaration_block("struct Point { x: i64 }"));
        assert!(is_declaration_block(
            "impl Point { fn x(self) -> i64 { self.x } }"
        ));
        assert!(!is_declaration_block("let x = 1;"));
        assert!(!is_declaration_block("print(1);"));
    }
}
