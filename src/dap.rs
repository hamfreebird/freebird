//! Freebird DAP 调试适配器（M4-2）。
//!
//! 通过 stdio 的 Debug Adapter Protocol 提供：
//! launch、setBreakpoints、configurationDone、stackTrace、scopes、variables、
//! continue / next / stepIn / stepOut、disconnect/terminate。
//!
//! 调试执行基于 VM 的源码行标记（`MirInst::Line`）与断点/单步控制；
//! 目前是单线程适配器，运行期间不接受 `pause`。

use std::path::Path;

use serde_json::{Value, json};

use crate::lsp::{read_message, write_message};
use crate::vm::Vm;
use crate::vm::config::RuntimeConfig;

pub struct DapServer {
    vm: Option<Vm>,
    breakpoints: Vec<u32>,
    seq: i64,
    source_path: Option<String>,
    source_name: String,
    stop_on_entry: bool,
    exit_requested: bool,
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
        }
    }

    pub fn run_stdio(&mut self) -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let stdout = std::io::stdout();
        let mut writer = stdout.lock();

        while let Some(message) = read_message(&mut reader)? {
            let responses = self.handle(&message);
            for response in responses {
                write_message(&mut writer, &response)?;
            }
            if self.exit_requested {
                break;
            }
        }
        Ok(())
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
                let response = self.response(message, true, json!({}), None);
                let mut messages = vec![response];
                let reason = if self.stop_on_entry {
                    "entry"
                } else {
                    "breakpoint"
                };
                messages.extend(self.run_action(message, DebugAction::Start, reason));
                messages
            }
            "threads" => vec![self.response(
                message,
                true,
                json!({ "threads": [{ "id": 1, "name": "main" }] }),
                None,
            )],
            "stackTrace" => vec![self.response(message, true, self.stack_trace(), None)],
            "scopes" => vec![self.response(message, true, self.scopes(message), None)],
            "variables" => vec![self.response(message, true, self.variables(message), None)],
            "continue" => {
                let response =
                    self.response(message, true, json!({ "allThreadsContinued": true }), None);
                let mut messages = vec![response];
                messages.extend(self.run_action(message, DebugAction::Continue, "breakpoint"));
                messages
            }
            "next" | "stepIn" => {
                let response = self.response(message, true, json!({}), None);
                let mut messages = vec![response];
                messages.extend(self.run_action(message, DebugAction::Step, "step"));
                messages
            }
            "stepOut" => {
                let response = self.response(message, true, json!({}), None);
                let mut messages = vec![response];
                messages.extend(self.run_action(message, DebugAction::StepOut, "step"));
                messages
            }
            "disconnect" | "terminate" => {
                let response = self.response(message, true, json!({}), None);
                self.vm = None;
                self.exit_requested = true;
                vec![response, self.event("terminated", json!({}))]
            }
            "pause" => vec![self.response(
                message,
                false,
                Value::Null,
                Some("pause is not supported by the single-threaded adapter"),
            )],
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

    fn run_action(&mut self, _message: &Value, action: DebugAction, reason: &str) -> Vec<Value> {
        let breakpoints = self.breakpoints.clone();
        let Some(vm) = self.vm.as_mut() else {
            return vec![self.event(
                "output",
                json!({ "category": "stderr", "output": "no active debug session\n" }),
            )];
        };
        vm.set_breakpoints(breakpoints);
        let result = match action {
            DebugAction::Continue => vm.continue_debug(),
            DebugAction::Step => vm.step_debug(),
            DebugAction::StepOut => vm.step_out_debug(),
            DebugAction::Start => vm.run_debug(),
        };
        let paused = vm.paused;
        let finished = vm.finished;
        let output = if finished {
            vm.take_output()
        } else {
            Vec::new()
        };

        match result {
            Ok(()) if paused => vec![self.event(
                "stopped",
                json!({ "reason": reason, "threadId": 1, "allThreadsStopped": true }),
            )],
            Ok(()) => self.finish_events(output, 0),
            Err(error) => {
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
        let top_index = vm.call_stack.len().saturating_sub(1);
        let frames: Vec<Value> = vm
            .call_stack
            .iter()
            .enumerate()
            .rev()
            .map(|(index, frame)| {
                json!({
                    "id": index + 1,
                    "name": frame.fn_name.clone().unwrap_or_else(|| format!("frame{index}")),
                    "source": { "name": self.source_name, "path": self.source_path },
                    "line": if index == top_index { vm.current_line.max(1) } else { 1 },
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

    fn variables(&self, message: &Value) -> Value {
        let reference = message["arguments"]["variablesReference"]
            .as_u64()
            .unwrap_or(0) as usize;
        let Some(vm) = self.vm.as_ref() else {
            return json!({ "variables": [] });
        };
        if reference == 0 || reference > vm.call_stack.len() {
            return json!({ "variables": [] });
        }
        let frame = &vm.call_stack[reference - 1];
        let mut variables = Vec::new();
        for (name, id) in frame.vars.current_frame_bindings() {
            if let Some(value) = frame.data.value_of(id) {
                variables.push(json!({
                    "name": name,
                    "value": value.to_string(),
                    "type": value.type_name(),
                    "variablesReference": 0
                }));
            }
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
}
