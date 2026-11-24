//! Freebird DAP 调试适配器（M4-2）。
//!
//! 通过 stdio 的 Debug Adapter Protocol 提供：
//! launch、setBreakpoints、configurationDone、stackTrace、scopes、variables、
//! continue / next / stepIn / stepOut、pause、disconnect/terminate。
//!
//! 调试执行基于 VM 的源码行标记（`MirInst::Line`）与断点/单步控制。
//! `run_stdio` 用独立线程读取 stdin，主循环按分片（[`ADVANCE_STEPS`] 条指令）
//! 推进 VM，因此运行期间也能响应 `pause`（协作式暂停点，不是抢占式线程）。

use std::path::Path;
use std::sync::mpsc::TryRecvError;

use serde_json::{Value, json};

use crate::lsp::{read_message, write_message};
use crate::vm::Vm;
use crate::vm::config::RuntimeConfig;
use crate::vm::value::Value as VmValue;

/// 每个执行分片的 MIR 指令数：足够大以保持调试吞吐，又足够小以便及时响应 `pause`。
const ADVANCE_STEPS: u64 = 20_000;

/// 复合值展开引用号的基准：`EXPAND_BASE + i` 指向 `expansions[i]`。
///
/// 帧引用号是 `1..=call_stack.len()`，两者不会冲突。
const EXPAND_BASE: usize = 1_000_000;

/// 变量值预览的最大字符数（数组/结构体可能很长）。
const PREVIEW_CHARS: usize = 80;

/// 该值是否有可展开的成员（数组/结构体/枚举载荷/Vec 都是 `Tuple`，`Box` 是 `Boxed`）。
fn is_expandable(value: &VmValue) -> bool {
    match value {
        VmValue::Tuple(elems) => !elems.is_empty(),
        VmValue::Boxed(_) => true,
        _ => false,
    }
}

/// 值预览：截断过长的复合值表示。
fn preview(value: &VmValue) -> String {
    let text = value.to_string();
    if text.chars().count() > PREVIEW_CHARS {
        let mut short: String = text.chars().take(PREVIEW_CHARS).collect();
        short.push('…');
        short
    } else {
        text
    }
}

pub struct DapServer {
    vm: Option<Vm>,
    breakpoints: Vec<u32>,
    seq: i64,
    source_path: Option<String>,
    source_name: String,
    stop_on_entry: bool,
    exit_requested: bool,
    /// 运行中的停止原因（`breakpoint`/`step`/`entry`）；`None` 表示已停住。
    running: Option<String>,
    /// 收到 `pause`：下一个分片边界立即暂停。
    pending_pause: bool,
    /// `run_stdio` 模式：运行类请求只启动，由主循环分片推进（否则同步跑到停下）。
    deferred: bool,
    /// 复合值的展开表：`variables` 请求按需登记，引用号为 `EXPAND_BASE + index`。
    ///
    /// 每次停下（stopped）时清空，保证一次暂停期间的引用集合稳定。
    expansions: Vec<Vec<(String, VmValue)>>,
}

enum DebugAction {
    Continue,
    Step,
    StepOut,
    Start,
}

impl DapServer {
    pub fn new() -> Self {
        Self {
            vm: None,
            breakpoints: Vec::new(),
            seq: 0,
            source_path: None,
            source_name: "program.fb".to_string(),
            stop_on_entry: false,
            exit_requested: false,
            running: None,
            pending_pause: false,
            deferred: false,
            expansions: Vec::new(),
        }
    }

    pub fn run_stdio(&mut self) -> std::io::Result<()> {
        // stdin 用独立线程读取：主线程在执行分片之间能继续处理 `pause`。
        let (sender, receiver) = std::sync::mpsc::channel::<Value>();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut reader = stdin.lock();
            while let Ok(Some(message)) = read_message(&mut reader) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });

        let stdout = std::io::stdout();
        let mut writer = stdout.lock();
        self.deferred = true;

        loop {
            // 有运行中的动作：先推进一个分片，再处理到达的请求。
            if self.running.is_some() {
                let events = self.advance(ADVANCE_STEPS);
                for event in events {
                    write_message(&mut writer, &event)?;
                }
                if self.exit_requested {
                    break;
                }
                if self.running.is_some() {
                    match receiver.try_recv() {
                        Ok(message) => {
                            if self.dispatch(&message, &mut writer)? {
                                break;
                            }
                        }
                        Err(TryRecvError::Empty) => continue,
                        Err(TryRecvError::Disconnected) => {
                            self.running = None;
                        }
                    }
                }
                continue;
            }

            match receiver.recv() {
                Ok(message) => {
                    if self.dispatch(&message, &mut writer)? {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        Ok(())
    }

    /// 处理一条消息并写出所有响应/事件；返回是否应结束会话。
    fn dispatch(
        &mut self,
        message: &Value,
        writer: &mut impl std::io::Write,
    ) -> std::io::Result<bool> {
        let responses = self.handle(message);
        for response in responses {
            write_message(writer, &response)?;
        }
        Ok(self.exit_requested)
    }

    pub fn handle(&mut self, message: &Value) -> Vec<Value> {
        let command = message
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default();

        match command {
            "initialize" => {
                let response = self.response(
                    message,
                    true,
                    json!({
                        "supportsConfigurationDoneRequest": true,
                        "supportsTerminateRequest": true,
                        "supportsStepBack": false,
                        "supportsSingleThreadExecutionRequests": false
                    }),
                    None,
                );
                let initialized = self.event("initialized", json!({}));
                vec![response, initialized]
            }
            "launch" => vec![self.launch(message)],
            "setBreakpoints" => vec![self.set_breakpoints_request(message)],
            "configurationDone" => {
                let reason = if self.stop_on_entry {
                    "entry"
                } else {
                    "breakpoint"
                };
                self.start_action(message, DebugAction::Start, reason)
            }
            "threads" => vec![self.response(
                message,
                true,
                json!({ "threads": [{ "id": 1, "name": "main" }] }),
                None,
            )],
            "stackTrace" => vec![self.response(message, true, self.stack_trace(), None)],
            "scopes" => vec![self.response(message, true, self.scopes(message), None)],
            "variables" => {
                let body = self.variables(message);
                vec![self.response(message, true, body, None)]
            }
            "continue" => self.start_action(message, DebugAction::Continue, "breakpoint"),
            "next" | "stepIn" => self.start_action(message, DebugAction::Step, "step"),
            "stepOut" => self.start_action(message, DebugAction::StepOut, "step"),
            "disconnect" | "terminate" => {
                let response = self.response(message, true, json!({}), None);
                self.vm = None;
                self.running = None;
                self.exit_requested = true;
                vec![response, self.event("terminated", json!({}))]
            }
            "pause" => {
                if self.vm.is_none() {
                    return vec![self.response(
                        message,
                        false,
                        Value::Null,
                        Some("no active debug session"),
                    )];
                }
                let response = self.response(message, true, json!({}), None);
                if self.running.is_some() {
                    // 运行中：在下一个分片边界停下（stopped 事件由 `advance` 发出）。
                    self.pending_pause = true;
                } else if let Some(vm) = self.vm.as_mut() {
                    // 已经停住：确认请求即可，不重复发 stopped 事件。
                    vm.paused = true;
                }
                vec![response]
            }
            other => vec![self.response(
                message,
                false,
                Value::Null,
                Some(&format!("unsupported DAP command: {other}")),
            )],
        }
    }

    fn launch(&mut self, message: &Value) -> Value {
        let arguments = &message["arguments"];
        let program = arguments
            .get("program")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if program.is_empty() {
            return self.response(message, false, Value::Null, Some("missing `program` path"));
        }
        let source = match std::fs::read_to_string(program) {
            Ok(source) => source,
            Err(error) => {
                return self.response(
                    message,
                    false,
                    Value::Null,
                    Some(&format!("failed to read `{program}`: {error}")),
                );
            }
        };
        let compiled = match crate::pipeline::compile_program(&source) {
            Ok(compiled) => compiled,
            Err(error) => {
                return self.response(message, false, Value::Null, Some(&error));
            }
        };
        let entry = compiled
            .entry
            .or_else(|| compiled.functions.first().map(|(id, _)| *id));
        let Some(main_mir) = entry.and_then(|entry| {
            compiled
                .functions
                .iter()
                .find(|(id, _)| *id == entry)
                .map(|(_, mir)| mir.clone())
        }) else {
            return self.response(
                message,
                false,
                Value::Null,
                Some("program has no entry function"),
            );
        };

        let mut vm = Vm::with_config(RuntimeConfig::test());
        for (id, mir) in &compiled.functions {
            vm.register_fn(*id, mir.clone());
        }
        if let Err(error) = vm.prepare_mir(main_mir) {
            return self.response(message, false, Value::Null, Some(&error.to_string()));
        }

        self.stop_on_entry = arguments
            .get("stopOnEntry")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.source_path = Some(program.to_string());
        self.source_name = Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("program.fb")
            .to_string();
        self.vm = Some(vm);
        self.response(message, true, json!({}), None)
    }

    fn set_breakpoints_request(&mut self, message: &Value) -> Value {
        let lines: Vec<u32> = message["arguments"]["breakpoints"]
            .as_array()
            .map(|breakpoints| {
                breakpoints
                    .iter()
                    .filter_map(|breakpoint| {
                        breakpoint.get("line")?.as_u64().map(|line| line as u32)
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.breakpoints = lines.clone();
        if let Some(vm) = self.vm.as_mut() {
            vm.set_breakpoints(lines.clone());
        }
        let breakpoints: Vec<Value> = lines
            .iter()
            .map(|line| json!({ "verified": true, "line": line }))
            .collect();
        self.response(message, true, json!({ "breakpoints": breakpoints }), None)
    }

    /// 发起一次运行类动作：设置 VM 模式并进入运行状态。
    ///
    /// `run_stdio`（deferred）下只启动，由主循环分片推进；同步调用（测试）
    /// 下直接推进到停下，保持"一次调用返回 stopped / finished 事件"的语义。
    fn start_action(&mut self, message: &Value, action: DebugAction, reason: &str) -> Vec<Value> {
        let body = match action {
            DebugAction::Continue => json!({ "allThreadsContinued": true }),
            _ => json!({}),
        };
        let response = self.response(message, true, body, None);
        let breakpoints = self.breakpoints.clone();
        let Some(vm) = self.vm.as_mut() else {
            return vec![self.event(
                "output",
                json!({ "category": "stderr", "output": "no active debug session\n" }),
            )];
        };
        vm.set_breakpoints(breakpoints);
        match action {
            DebugAction::Start => vm.arm_start(),
            DebugAction::Continue => vm.arm_continue(),
            DebugAction::Step => vm.arm_step(),
            DebugAction::StepOut => vm.arm_step_out(),
        }
        self.running = Some(reason.to_string());

        let mut messages = vec![response];
        if !self.deferred {
            messages.extend(self.advance(u64::MAX));
        }
        messages
    }

    /// 推进一个执行分片，返回本次产生的 DAP 事件。
    ///
    /// 分片用尽而程序仍在运行时返回空事件，由 `run_stdio` 的主循环继续推进。
    fn advance(&mut self, max_steps: u64) -> Vec<Value> {
        let Some(reason) = self.running.clone() else {
            return Vec::new();
        };
        // `pause`：在分片边界停下，而不是等下一个断点。
        if self.pending_pause {
            self.pending_pause = false;
            self.running = None;
            self.expansions.clear();
            if let Some(vm) = self.vm.as_mut() {
                vm.paused = true;
            }
            return vec![self.event(
                "stopped",
                json!({ "reason": "pause", "threadId": 1, "allThreadsStopped": true }),
            )];
        }
        let Some(vm) = self.vm.as_mut() else {
            self.running = None;
            return vec![self.event(
                "output",
                json!({ "category": "stderr", "output": "no active debug session\n" }),
            )];
        };
        let result = vm.run_debug_slice(max_steps);
        let paused = vm.paused;
        let finished = vm.finished;
        let output = if finished {
            vm.take_output()
        } else {
            Vec::new()
        };

        match result {
            Ok(()) if paused => {
                self.running = None;
                self.expansions.clear();
                vec![self.event(
                    "stopped",
                    json!({ "reason": reason, "threadId": 1, "allThreadsStopped": true }),
                )]
            }
            Ok(()) if finished => {
                self.running = None;
                self.finish_events(output, 0)
            }
            // 分片预算用尽，程序仍在运行。
            Ok(()) => Vec::new(),
            Err(error) => {
                self.running = None;
                let mut messages = vec![self.event(
                    "output",
                    json!({ "category": "stderr", "output": format!("{error}\n") }),
                )];
                messages.extend(self.finish_events(output, 1));
                messages
            }
        }
    }

    fn finish_events(&mut self, output: Vec<u8>, exit_code: i64) -> Vec<Value> {
        let mut events = Vec::new();
        if !output.is_empty() {
            events.push(self.event(
                "output",
                json!({
                    "category": "stdout",
                    "output": String::from_utf8_lossy(&output)
                }),
            ));
        }
        events.push(self.event("exited", json!({ "exitCode": exit_code })));
        events.push(self.event("terminated", json!({})));
        self.vm = None;
        events
    }

    fn stack_trace(&self) -> Value {
        let Some(vm) = self.vm.as_ref() else {
            return json!({ "stackFrames": [], "totalFrames": 0 });
        };
        let frames: Vec<Value> = vm
            .call_stack
            .iter()
            .enumerate()
            .rev()
            .map(|(index, frame)| {
                // 每一帧都带自己的当前行；0 表示该帧尚未执行到任何语句。
                json!({
                    "id": index + 1,
                    "name": frame.fn_name.clone().unwrap_or_else(|| format!("frame{index}")),
                    "source": { "name": self.source_name, "path": self.source_path },
                    "line": frame.current_line.max(1),
                    "column": 1
                })
            })
            .collect();
        json!({ "totalFrames": frames.len(), "stackFrames": frames })
    }

    fn scopes(&self, message: &Value) -> Value {
        let frame_id = message["arguments"]["frameId"].as_u64().unwrap_or(1) as usize;
        let Some(vm) = self.vm.as_ref() else {
            return json!({ "scopes": [] });
        };
        if frame_id == 0 || frame_id > vm.call_stack.len() {
            return json!({ "scopes": [] });
        }
        json!({
            "scopes": [{
                "name": "Locals",
                "variablesReference": frame_id,
                "expensive": false
            }]
        })
    }

    /// 登记一个复合值的展开项，返回 `variables[]` 可用的引用号。
    fn register_expansion(&mut self, value: &VmValue) -> usize {
        let children: Vec<(String, VmValue)> = match value {
            // 数组/结构体/枚举载荷/Vec 在 VM 里都是 `Tuple` 单槽值。
            VmValue::Tuple(elems) => elems
                .iter()
                .enumerate()
                .map(|(index, element)| (format!("[{index}]"), element.clone()))
                .collect(),
            VmValue::Boxed(inner) => vec![("*".to_string(), (**inner).clone())],
            _ => Vec::new(),
        };
        self.expansions.push(children);
        EXPAND_BASE + self.expansions.len() - 1
    }

    /// 生成单个变量的 DAP 描述；复合值附带展开引用号。
    fn describe_variable(&mut self, name: &str, value: &VmValue) -> Value {
        let variables_reference = if is_expandable(value) {
            self.register_expansion(value)
        } else {
            0
        };
        json!({
            "name": name,
            "value": preview(value),
            "type": value.type_name(),
            "variablesReference": variables_reference
        })
    }

    fn variables(&mut self, message: &Value) -> Value {
        let reference = message["arguments"]["variablesReference"]
            .as_u64()
            .unwrap_or(0) as usize;
        if reference == 0 {
            return json!({ "variables": [] });
        }

        // 复合值展开：返回子成员（`[0]`/`[1]`/`*` 等），子成员可继续展开。
        if reference >= EXPAND_BASE {
            let index = reference - EXPAND_BASE;
            let Some(children) = self.expansions.get(index).cloned() else {
                return json!({ "variables": [] });
            };
            let mut variables = Vec::new();
            for (label, value) in children {
                variables.push(self.describe_variable(&label, &value));
            }
            return json!({ "variables": variables });
        }

        // 帧级局部变量。
        let Some(vm) = self.vm.as_ref() else {
            return json!({ "variables": [] });
        };
        if reference > vm.call_stack.len() {
            return json!({ "variables": [] });
        }
        let frame = &vm.call_stack[reference - 1];
        let bindings: Vec<(String, VmValue)> = frame
            .vars
            .current_frame_bindings()
            .into_iter()
            .filter_map(|(name, id)| frame.data.value_of(id).map(|value| (name, value.clone())))
            .collect();

        let mut variables = Vec::new();
        for (name, value) in bindings {
            variables.push(self.describe_variable(&name, &value));
        }
        json!({ "variables": variables })
    }

    fn response(
        &mut self,
        request: &Value,
        success: bool,
        body: Value,
        message: Option<&str>,
    ) -> Value {
        self.seq += 1;
        let mut response = json!({
            "seq": self.seq,
            "type": "response",
            "request_seq": request.get("seq").and_then(Value::as_i64).unwrap_or(0),
            "success": success,
            "command": request.get("command").and_then(Value::as_str).unwrap_or_default()
        });
        if !body.is_null() {
            response["body"] = body;
        }
        if let Some(message) = message {
            response["message"] = json!(message);
        }
        response
    }

    fn event(&mut self, event: &str, body: Value) -> Value {
        self.seq += 1;
        json!({ "seq": self.seq, "type": "event", "event": event, "body": body })
    }
}

impl Default for DapServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(seq: i64, command: &str, arguments: Value) -> Value {
        json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments
        })
    }

    fn temp_program() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "freebird-dap-{}-{}.fb",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            "fn main() {\n    let a = 1;\n    let b = 2;\n    print(a + b);\n}\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn breakpoint_stack_and_variables() {
        let path = temp_program();
        let mut server = DapServer::new();
        let initialize = server.handle(&request(1, "initialize", json!({})));
        assert_eq!(initialize.len(), 2);
        assert_eq!(initialize[1]["event"], "initialized");

        let launch = server.handle(&request(
            2,
            "launch",
            json!({ "program": path.to_string_lossy() }),
        ));
        assert_eq!(launch[0]["success"], true);

        let breakpoints = server.handle(&request(
            3,
            "setBreakpoints",
            json!({ "breakpoints": [{ "line": 3 }] }),
        ));
        assert_eq!(breakpoints[0]["body"]["breakpoints"][0]["verified"], true);

        let configuration = server.handle(&request(4, "configurationDone", json!({})));
        assert_eq!(configuration[0]["success"], true);
        assert_eq!(configuration[1]["event"], "stopped");

        let stack = server.handle(&request(5, "stackTrace", json!({ "threadId": 1 })));
        let frame = &stack[0]["body"]["stackFrames"][0];
        assert_eq!(frame["name"], "main");
        assert_eq!(frame["line"], 3);

        let variables = server.handle(&request(
            6,
            "variables",
            json!({ "variablesReference": frame["id"] }),
        ));
        let locals = variables[0]["body"]["variables"].as_array().unwrap();
        assert!(
            locals
                .iter()
                .any(|v| v["name"] == "a" && v["type"] == "i64")
        );

        let continued = server.handle(&request(7, "continue", json!({ "threadId": 1 })));
        assert_eq!(continued[0]["success"], true);
        assert!(continued.iter().any(|message| message["event"] == "exited"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn step_reaches_next_line() {
        let path = temp_program();
        let mut server = DapServer::new();
        server.handle(&request(1, "initialize", json!({})));
        server.handle(&request(
            2,
            "launch",
            json!({ "program": path.to_string_lossy() }),
        ));
        server.handle(&request(
            3,
            "setBreakpoints",
            json!({ "breakpoints": [{ "line": 3 }] }),
        ));
        server.handle(&request(4, "configurationDone", json!({})));
        let stepped = server.handle(&request(5, "next", json!({ "threadId": 1 })));
        assert!(stepped.iter().any(|message| message["event"] == "stopped"));
        let stack = server.handle(&request(6, "stackTrace", json!({})));
        let line = stack[0]["body"]["stackFrames"][0]["line"].as_u64().unwrap();
        assert_eq!(line, 3);
        let _ = std::fs::remove_file(path);
    }

    /// 写一个临时程序文件（带唯一后缀）。
    fn temp_program_with_source(tag: &str, source: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "freebird-dap-{tag}-{}-{}.fb",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, source).unwrap();
        path
    }

    /// 长循环程序：分片推进时难以一次跑完，便于在运行中插入 `pause`。
    fn temp_long_program() -> std::path::PathBuf {
        temp_program_with_source(
            "long",
            "fn main() {\n    let mut i = 0;\n    while i < 1000000 {\n        i = i + 1;\n    }\n    print(i);\n}\n",
        )
    }

    #[test]
    fn variables_expand_composite_values() {
        let path = temp_program_with_source(
            "expand",
            "struct Point { x: i64, y: i64 }\n\nfn main() {\n    let a = [1, 2, 3];\n    let p = Point { x: 7, y: 8 };\n    let v = vec![4, 5];\n    print(a[0] + p.x + v[0]);\n}\n",
        );
        let mut server = DapServer::new();
        server.handle(&request(1, "initialize", json!({})));
        server.handle(&request(
            2,
            "launch",
            json!({ "program": path.to_string_lossy() }),
        ));
        server.handle(&request(
            3,
            "setBreakpoints",
            json!({ "breakpoints": [{ "line": 7 }] }),
        ));
        let configured = server.handle(&request(4, "configurationDone", json!({})));
        assert!(
            configured.iter().any(|m| m["event"] == "stopped"),
            "{configured:?}"
        );

        let stack = server.handle(&request(5, "stackTrace", json!({})));
        let frame_id = stack[0]["body"]["stackFrames"][0]["id"].as_u64().unwrap();
        let locals = server.handle(&request(
            6,
            "variables",
            json!({ "variablesReference": frame_id }),
        ));
        let locals = locals[0]["body"]["variables"].as_array().unwrap().clone();
        let find = |name: &str| {
            locals
                .iter()
                .find(|variable| variable["name"] == name)
                .cloned()
                .unwrap_or_else(|| panic!("missing local `{name}` in {locals:?}"))
        };

        // 数组：三个元素可展开
        let array = find("a");
        assert!(
            array["variablesReference"].as_u64().unwrap() > 0,
            "array should be expandable: {array:?}"
        );
        let elements = server.handle(&request(
            7,
            "variables",
            json!({ "variablesReference": array["variablesReference"] }),
        ));
        let elements = elements[0]["body"]["variables"].as_array().unwrap().clone();
        assert_eq!(elements.len(), 3, "{elements:?}");
        assert_eq!(elements[0]["name"], "[0]");

        // 结构体：字段按位置展开
        let point = find("p");
        let fields = server.handle(&request(
            8,
            "variables",
            json!({ "variablesReference": point["variablesReference"] }),
        ));
        let fields = fields[0]["body"]["variables"].as_array().unwrap().clone();
        assert_eq!(fields.len(), 2, "{fields:?}");
        assert!(
            fields[0]["value"].as_str().unwrap().starts_with('7'),
            "{fields:?}"
        );

        // Vec：同样可展开
        let vec_var = find("v");
        let items = server.handle(&request(
            9,
            "variables",
            json!({ "variablesReference": vec_var["variablesReference"] }),
        ));
        assert_eq!(items[0]["body"]["variables"].as_array().unwrap().len(), 2);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stack_trace_reports_line_per_frame() {
        let path = temp_program_with_source(
            "frames",
            "fn inner(x: i64) -> i64 {\n    let y = x + 1;\n    y\n}\n\nfn main() {\n    let a = inner(1);\n    print(a);\n}\n",
        );
        let mut server = DapServer::new();
        server.handle(&request(1, "initialize", json!({})));
        server.handle(&request(
            2,
            "launch",
            json!({ "program": path.to_string_lossy() }),
        ));
        server.handle(&request(
            3,
            "setBreakpoints",
            json!({ "breakpoints": [{ "line": 2 }] }),
        ));
        let configured = server.handle(&request(4, "configurationDone", json!({})));
        assert!(
            configured.iter().any(|m| m["event"] == "stopped"),
            "{configured:?}"
        );

        let stack = server.handle(&request(5, "stackTrace", json!({})));
        let frames = stack[0]["body"]["stackFrames"].as_array().unwrap();
        assert_eq!(frames.len(), 2, "{frames:?}");
        // 顶层是 inner（停在断点第 2 行），调用者是 main（停在第 7 行的调用点）。
        assert_eq!(frames[0]["line"], 2);
        assert_eq!(frames[1]["line"], 7);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pause_stops_running_program() {
        let path = temp_long_program();
        let mut server = DapServer::new();
        server.handle(&request(1, "initialize", json!({})));
        server.handle(&request(
            2,
            "launch",
            json!({ "program": path.to_string_lossy() }),
        ));
        // 模拟 stdio 模式：运行类请求只启动，由分片推进。
        server.deferred = true;
        let started = server.handle(&request(3, "configurationDone", json!({})));
        assert_eq!(started.len(), 1, "deferred 模式下只返回响应: {started:?}");
        assert!(server.running.is_some(), "configurationDone 应进入运行状态");

        // 推进几个分片后程序仍在运行（没有断点，不会自己停下）。
        for _ in 0..3 {
            let events = server.advance(1_000);
            assert!(
                !events.iter().any(|message| message["event"] == "stopped"),
                "不应在 pause 之前停下: {events:?}"
            );
            if server.running.is_none() {
                break;
            }
        }
        assert!(server.running.is_some(), "程序应仍在运行");

        // 运行中 `pause`：请求立即成功，暂停事件在下一个分片边界发出。
        let pause = server.handle(&request(4, "pause", json!({ "threadId": 1 })));
        assert_eq!(pause[0]["success"], true);
        let stopped = server.advance(1_000);
        assert!(
            stopped.iter().any(
                |message| message["event"] == "stopped" && message["body"]["reason"] == "pause"
            ),
            "应发出 reason=pause 的 stopped 事件: {stopped:?}"
        );
        assert!(server.running.is_none(), "pause 后应退出运行状态");

        // 暂停后仍可查看调用栈。
        let stack = server.handle(&request(5, "stackTrace", json!({ "threadId": 1 })));
        assert!(
            !stack[0]["body"]["stackFrames"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        // 已暂停时的 `pause` 也返回成功，且不重复发事件。
        let pause_again = server.handle(&request(6, "pause", json!({ "threadId": 1 })));
        assert_eq!(pause_again.len(), 1);
        assert_eq!(pause_again[0]["success"], true);
        let _ = std::fs::remove_file(path);
    }
}
