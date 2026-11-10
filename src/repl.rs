//! Freebird REPL — 交互式读取-求值-打印循环
//!
//! 支持：:stack, :vars, :info, :quit/:q, :help 指令

use std::io::{self, BufRead, Write};

use crate::frontend::parse;
use crate::hir::lower;
use crate::mir::MirGen;
use crate::typeck::TypeTable;
use crate::vm::config::RuntimeConfig;
use crate::vm::Vm;

/// REPL 状态
pub struct Repl {
    vm: Vm,
    line_count: usize,
    fn_counter: u64,
    /// 累积的函数声明
    accumulated_decls: String,
}

impl Repl {
    pub fn new() -> Self {
        Self {
            vm: Vm::with_config(RuntimeConfig::test()),
            line_count: 0,
            fn_counter: 1,
            accumulated_decls: String::new(),
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
            print!("fb> ");
            io::stdout().flush()?;

            input.clear();
            if reader.read_line(&mut input)? == 0 {
                break; // EOF
            }

            let line = input.trim();
            if line.is_empty() {
                continue;
            }

            // 处理特殊命令
            if line.starts_with(':') {
                match line {
                    ":q" | ":quit" => break,
                    ":help" => self.cmd_help(),
                    ":stack" => self.cmd_stack(),
                    ":vars" => self.cmd_vars(),
                    ":info" => self.cmd_info(),
                    ":clear" => { self.accumulated_decls.clear(); println!("Cleared."); }
                    _ => println!("Unknown command: {line}. Type :help for commands."),
                }
                continue;
            }

            // 尝试作为函数声明或表达式执行
            self.execute_line(line);
        }

        println!("\nBye!");
        Ok(())
    }

    fn execute_line(&mut self, line: &str) {
        // 检查是否为函数声明
        let is_fn_decl = line.trim_start().starts_with("fn ")
            || line.trim_start().starts_with("struct ")
            || line.trim_start().starts_with("enum ")
            || line.trim_start().starts_with("extern ")
            || line.trim_start().starts_with("use ");

        if is_fn_decl {
            self.accumulated_decls.push_str(line);
            self.accumulated_decls.push('\n');
            println!("  [declared]");
            return;
        }

        // 表达式/语句 — 包装在 main 函数中执行
        let source = if self.accumulated_decls.is_empty() {
            format!("fn _repl_main() {{ {} }}", line)
        } else {
            format!("{}\nfn _repl_main() {{ {} }}", self.accumulated_decls, line)
        };

        let (ast, ast_errors) = parse(&source);
        if !ast_errors.is_empty() {
            for e in &ast_errors {
                eprintln!("  parse error: {e}");
            }
            return;
        }

        let (hir, hir_errors) = lower(&ast);
        if !hir_errors.is_empty() {
            for e in &hir_errors {
                eprintln!("  resolve error: {e}");
            }
            return;
        }

        let type_table = TypeTable::from_ast_decls(&ast.decls);
        let mut generator = MirGen::new().with_type_table(type_table);
        let functions = generator.gen_program(&hir);

        // 找到 _repl_main 函数
        let main_mir = functions.iter()
            .find(|(_, mir)| !mir.is_empty())
            .map(|(_, mir)| mir.clone());

        match main_mir {
            Some(mir) => {
                // 注册所有函数
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
                        // 显示执行结果（栈顶值）
                        if let Some(frame) = self.vm.call_stack.last() {
                            if let Some(val) = frame.data.top_value() {
                                println!("  => {val}");
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("  runtime error: {e}");
                    }
                }
            }
            None => {
                eprintln!("  no executable code generated");
            }
        }
    }

    fn cmd_help(&self) {
        println!("Commands:");
        println!("  :stack  — display current stack contents");
        println!("  :vars   — display current variable bindings");
        println!("  :info   — display VM state info");
        println!("  :clear  — clear accumulated declarations");
        println!("  :q/:quit — exit REPL");
        println!("  :help   — show this help");
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
        match self.vm.call_stack.last() {
            Some(frame) => {
                let bindings = frame.vars.current_frame_bindings();
                if bindings.is_empty() {
                    println!("  (no variables)");
                } else {
                    println!("Variables:");
                    for (name, id) in &bindings {
                        println!("  {name} → {id:?}");
                    }
                }
            }
            None => println!("  (no active frame)"),
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
