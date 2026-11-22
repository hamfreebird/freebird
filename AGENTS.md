# AGENTS.md

Freebird is a single-crate Rust implementation of a stack-based language. `readme.md` is the intended spec, `USAGE.md` covers the CLI, and `IMPROVEMENTS.md` is a dated v3 audit of known gaps. Docs and code comments are Chinese; `readme.md` and `USAGE.md` describe features that are not fully implemented — trust `src/` and the tests when they conflict.

## Commands

- Build: `cargo build --release` (edition 2024, Rust 1.85+).
- Test: `cargo test` — current baseline is 204 unit + 61 integration tests, all green (`tests/integration_test.rs` 33 + `tests/m0_test.rs` 28).
- Single test: `cargo test <test_name>`; integration only: `cargo test --test integration_test test_fib_program`, `cargo test --test m0_test`.
- CLI: `cargo run -- run <file.fb>`, `cargo run -- check <file.fb>`, `cargo run -- build [--target stardust|llvm] <file.fb>`, `cargo run -- repl`.
- Run tests from the repo root: integration tests load fixtures via relative paths. `tests/programs/*.fb` are not auto-discovered (`enum_match.fb` is currently unused); add an explicit test for each new fixture.

## Toolchain gotchas

- CI (`.github/workflows/ci.yml`) runs `cargo build --release` and `cargo test`; there is no fmt/clippy gate. `cargo fmt --check` fails on the existing tree and `cargo clippy --all-targets` fails on a pre-existing `approx_constant` lint in `src/frontend/lexer.rs`. Do not reformat unrelated code or treat either as a required gate unless asked.
- Pipeline tests now assert output values for fib/while/struct/enum/`stardust!`; only the two `check_source` positive cases remain `is_ok()`. The Stardust integration tests compare interpreter output against `run_source` byte-for-byte. `tests/m0_test.rs` holds the M0 negative diagnostics and VM/Stardust differential cases; add output or VM-state assertions when changing behavior.
- `RuntimeConfig::test()` is the convention for program tests (max_stack 1024, max_call_depth 64, max_steps 1M, debug=true); CLI defaults are much larger. `run --debug` now selects `RuntimeConfig::debug()` and affects runtime errors.

## Architecture

- `src/pipeline.rs` is the wiring: `parse` → `TypeTable::from_ast_decls` → `lower` (HIR + name resolution) → `check_program` → `check_borrow` → `MirGen` → `Vm`. `run_source` executes; `build_stardust` performs the same checks and goes AST→HIR→MIR→Stardust text. `freebird check` uses the same parse/resolve/type/borrow checks and emits the same diagnostics as `run`/`build`.
- `run_source` applies `#[max_stack]`/`#[max_call_depth]`/`#[max_steps]`/`#[time_limit]` from `main` to its `RuntimeConfig`. Attribute syntax/name/value validation lives in `TypeChecker::validate_attributes`.
- The `MirInst` enum lives in `src/vm/mir.rs`, not `src/mir/`; MIR generation is `src/mir/generator.rs`; the interpreter is `src/vm/mod.rs`. The VM is a dual stack (ElementId + value); read the dual-stack and `roll`/variable semantics in `readme.md` §3–4 before changing stack or variable operations.
- `typeck` and `borrowck` are wired into `run_source` and `build_stardust`. Do not bypass them when adding new pipeline entry points.
- `build --target stardust` runs and covers numeric/char values, variables, control flow (`for`/`while`/`loop`, `break`/`continue`), calls, recursion, I/O, `ret n`, scalar `as` casts, and heap-backed arrays/structs/enums/strings/Box/Vec. `file_read`/`file_write` use the bundled interpreter's `FileRead`/`FileWrite` instructions (`,` with 2/3 leading spaces), passing strings as `[chars..., len]`. Remote stacks remain placeholder comments; `&mut` references are backed by heap cells, so writes through `*p = v` propagate back to the original variable (including when the reference is passed to another function). `--target llvm` exits with "not implemented".

## Language surface

- Builtins are registered from `src/io.rs::BUILTINS`: `print`/`print_int`, `print_char`, `print_str`, `println_str`, `read_int`, `read_char`, `read_line`, `file_read`, and `file_write`.
- Tuple destructuring (`let (a, b) = ...`), struct destructuring (`let Point { x, y } = p`), enum `match` with payload bindings, `vec![...]`, arrays, `Box`, indexing, and field/index assignment work in the VM. Mutable parameters (`fn f(mut x: T)`) are not part of the syntax.
- `match` supports struct, enum (including nested payloads), tuple, literal, `or`, reference and `_` patterns. Type checking rejects non-exhaustive matches, unknown variants/fields, `or` alternatives with different bindings, and refutable `let` patterns.
- `for pattern in iterable` iterates arrays, `Vec<T>`, `String` (yielding `char`), and `&Vec<T>`/`&String`; `continue` and `break` unwind loop-body scopes and stack slots correctly in both VM and Stardust backends. `ret n` requires an integer literal and arity matching the signature; `as` supports scalar numeric/`char`/`bool` conversions.
- M0 rejects `break value`, value-position `loop`, `#id`, `use`, and `extern` calls at compile time. `extern "stardust"` declarations and `#[stardust(id = N)]` parse and check, but FFI is not implemented.
- Stardust backend limits: remote stacks are placeholders; `read_line` decodes UTF-8 into a growable block and does not special-case EOF; runtime-limit attributes only affect the VM.

## I/O quirks

- `run_source` returns VM output bytes; the CLI prints them once. Assert against the returned bytes rather than process stdout.
- Stardust `read_line` decodes UTF-8 bytes into a growable character block (starts at 16 characters, doubles as needed); invalid leading bytes become U+FFFD and EOF is not special-cased. A line ends at `\n`, and trailing `\r` characters are trimmed, so CRLF input matches VM (`read_line` trims `\n`/`\r`).
- VM and Stardust both resolve `&mut` writes across call frames: `fn set(p: &mut i64) { *p = 42; }` propagates back to the caller's variable.
- `print(x); print(x)` works now: the builtin call is no longer treated as leaving an extra value on the stack.
