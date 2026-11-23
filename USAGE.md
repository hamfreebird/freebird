# Freebird 工具链使用指南

## 安装

### 从源码构建

```bash
# 要求 Rust 1.85+
git clone https://github.com/hamfreebird/freebird.git
cd freebird
cargo build --release
```

### VSCode 扩展安装

```bash
# 将扩展目录链接到 VSCode 扩展目录
ln -s $(pwd)/vscode ~/.vscode/extensions/freebird-language
# 重启 VSCode
```

## CLI 命令

### 运行程序

```bash
freebird run hello.fb          # 解释执行
freebird run --debug hello.fb  # 调试模式
```

### 静态检查

```bash
freebird check hello.fb        # JSON 诊断输出
```

`check` 与 `run`/`build` 执行同一套前端检查：`parse` → 名称解析 → 类型检查 → 借用/移动检查。发现问题时退出码为 1；诊断中包含行、列和修复建议。警告（如不可达 `match` 分支）以 `warning` 级别输出，JSON 包含 `error_count` 与 `warning_count`；只有错误会使退出码非 0。

### 编译

```bash
freebird build --target stardust hello.fb  # 编译为 Stardust 源码
```

`--target llvm` 尚未实现；请求该目标会直接报错退出。

## 当前实现状态

- VM 已支持数值/字符/布尔、变量与作用域、控制流、函数调用/递归、数组、结构体、枚举匹配、`Vec`、`Box`、字符串、元组解构以及 `read_*`/`print_*`/文件 I/O。
- 标准库（MVP）：内建 `Option<T>` / `Result<T, E>` 泛型枚举；`Option` 支持 `is_some`/`is_none`/`unwrap`/`unwrap_or`/`ok_or`，`Result` 支持 `is_ok`/`is_err`/`unwrap`/`unwrap_or`/`ok`/`err`；数学内建 `abs`/`min`/`max`；`Vec`/`String` 支持 `len`/`push`/`pop`/`is_empty`/`clear`。用户可为结构体/枚举声明 `impl Type { fn f(self) -> T { ... } }`：关联函数用 `Type::f(...)` 调用，值接收者方法用 `x.f(...)` 调用（`self.field` 访问字段）。`&self` 只读方法已支持（`fn area(&self)`），`&mut self` 尚未支持并报未实现错误。字符串转换提供 `int_to_str(i64) -> String`（双后端）与 `str_to_int(String) -> Option<i64>`（仅 VM；Stardust 后端生成显式陷阱）。`map` 等组合子尚未实现。
- 运行前会执行名称解析、类型检查和借用/移动检查；`check` 与 `run` 报告同一组前端错误。
- `match` 支持结构体、枚举（含嵌套载荷）、元组、字面量、`or`、引用、`..` 剩余和 `_` 模式，并在类型检查阶段拒绝非穷尽匹配；重复分支或兜底分支之后的分支会产生 `warning` 级诊断。`let` 支持变量、元组和结构体解构；可失败模式（如枚举变体）必须改用 `match`。
- `ret n` 的 `n` 必须是整数字面量；`loop` 支持值语义与 `break value`，`if let` / `while let` 已支持（VM/Stardust 输出一致）；`while`/`for` 中的 `break value` 会在编译期报错；循环标签 `'outer: loop` + `break 'outer` / `continue 'outer` 已支持。
- 函数指针：支持 `fn(A, B) -> C` 类型、函数名作值、作为参数/返回值以及间接调用；Stardust 通过 FnId dispatch 实现。当前 MVP 要求间接调用恰好返回一个值。
- `#id`、`use` 和 `extern` 函数调用尚未实现：声明可以解析，但使用处会报 `unsupported feature`，不会静默返回栈上残留值。
- Stardust 后端可生成并运行上述大部分程序，复杂值统一放在 Stardust 全局堆块上（逻辑远程栈，见 `docs/adr/0001-logical-remote-stack.md`）。`&mut` 引用使用堆单元间接访问，`*p = v` 会回写原变量（包括把引用传给函数后写入）。`for`/`continue`/`break`、`ret n`、标量 `as` 转换以及 `file_read`/`file_write` 均已支持；文件路径和内容按 UTF-8 文本处理，非 UTF-8/二进制内容不在支持范围内。
- `read_line` 与 VM 的 `String::from_utf8_lossy` 对齐：CRLF、UTF-8、非法首字节/续字节、overlong/surrogate/越界码点都逐字节一致。**已知差异**：输入 EOF 时 Stardust 解释器的 `CharIn` 直接报 I/O 错误（VM 返回空字符串），生成代码无法捕获。
- 索引读写在 Stardust 后端执行 `index >= 0 && index < len` 检查；越界时用除零陷阱让解释器非零退出，错误消息与 VM 的 `index out of bounds` 不同。
- Stardust 全局堆块使用 free list 复用；`VecPush` 原地追加、`VecPop` 原地缩减，扩容时释放旧块；`DropVar` 回收未被返回/别名的 Vec/Box 绑定。数组/结构体等 Copy 类型的 COW 旧块、嵌套容器内层块、从未 push/pop 的 Vec/String 保守不回收（安全泄漏）；完整引用计数方案见 `docs/adr/0002-heap-reclamation-deferred.md`。
- `stardust!` 的 `store`/`load` 映射为真实堆操作；`Reverse` 仍未支持（M3-4）。
- `#[max_stack]`/`#[max_call_depth]`/`#[max_steps]`/`#[time_limit]` 只影响 VM；生成的 Stardust 代码没有等价资源限制，由外部解释器负责。
- LLVM 后端尚未实现。
- 完整的能力矩阵见 `STATUS.md`；VM/Stardust 差分测试见 `tests/differential_test.rs` 与 `tests/m1_test.rs`。

### 交互式 REPL

```bash
freebird repl                  # 启动 REPL
```

REPL 命令：
- `:stack` — 显示当前栈内容（含 ElementId）
- `:vars` — 显示变量绑定
- `:info` — VM 状态信息
- `:clear` — 清除累积声明
- `:q` — 退出

## 语言示例

### 完整语言巡礼

`examples/tour.fb` 展示了字符串/Unicode、`String`、变量与遮蔽、`while`、递归、元组解构、结构体、枚举匹配、`Vec`、数组、`Box`、`&mut` 写回和 `stardust!` 宏：

```bash
cargo run -- run examples/tour.fb
cargo run -- build --target stardust examples/tour.fb > /tmp/tour.sd
./stardust/stardust-linux-x86_64 /tmp/tour.sd
```

### Hello World

```freebird
fn main() {
    stardust! {
        Push(72)    // 'H'
        Push(101)   // 'e'
        Push(108)   // 'l'
        Push(108)   // 'l'
        Push(111)   // 'o'
    }
}
```

### 多值返回

```freebird
fn divrem(a: i32, b: i32) -> (i32, i32) {
    return a / b, a % b;
}

fn main() {
    let (q, r) = divrem(10, 3);
    // q = 3, r = 1
}
```

### 结构体

```freebird
struct Point { x: i32, y: i32 }

fn distance(p: Point) -> i32 {
    return p.x * p.x + p.y * p.y;
}

fn main() {
    let p = Point { x: 3, y: 4 };
    let d = distance(p);
}
```

### 枚举与模式匹配

```freebird
struct Point { x: i64, y: i64 }
enum Option { None, Some(i32) }

fn map_opt(opt: Option) -> i32 {
    match opt {
        Option::None => 0,
        Option::Some(v) => v,
    }
}

fn describe(p: Point) -> i64 {
    let Point { x, y } = p;   // 结构体解构
    match x {
        0 => y,
        _ => x,
    }
}
```

匹配必须是穷尽的：枚举要覆盖全部变体，其他类型需要 `_` 或变量兜底。`or` 模式的各分支必须绑定相同变量；带 guard 的分支不计入穷尽性。`let` 只接受不可失败模式，枚举变体请使用 `match`。

### 可变变量与循环

```freebird
fn sum_to(n: i32) -> i32 {
    let mut total = 0;
    let mut i = 0;
    while i < n {
        i = i + 1;
        total = total + i;
    }
    return total;
}

fn sum_even(items: [i32; 6]) -> i32 {
    let mut total = 0;
    for x in items {
        if x % 2 != 0 {
            continue;
        }
        total = total + x;
    }
    return total;
}
```

`for` 可迭代数组、`Vec<T>` 和 `String`（元素为 `char`），也接受 `&Vec<T>`/`&String`。

### 类型转换与低级返回

```freebird
fn split(a: i64, b: i64) -> (i64, i64) {
    let d = a - b;
    let p = a * b;
    ret 2;                       // 保留栈顶两个值作为返回值
}

fn main() {
    let (d, _p) = split(7, 3);
    print(300 as u8);            // 44
    print_char(65 as char);      // A
}
```

`as` 只允许数值、`char` 和 `bool` 之间的标量转换；`ret n` 的 `n` 必须是整数字面量，且需与函数声明的返回值个数一致。

### 递归

```freebird
fn fib(n: i32) -> i32 {
    if n <= 1 { return n; }
    return fib(n - 1) + fib(n - 2);
}
```

### Unsafe + stardust! 宏

```freebird
fn raw_stack_op() -> i32 {
    unsafe {
        stardust! {
            Push(3)
            Push(5)
            Add
        }
    }
}
```

## 标准库

### IO 操作

```freebird
fn main() {
    print_int(42);           // 输出数字
    print_char('A');         // 输出字符
    let x = read_int();      // 读取数字
    let c = read_char();     // 读取字符
}
```

## 运行时配置与属性

通过 `#[key=value]` 或 `#[key(value)]` 注解覆盖运行时配置。目前从 `main` 函数上的属性读取，作用于 VM：

```freebird
#[max_steps=100000]
#[time_limit=2]
fn main() { /* ... */ }
```

| 配置 | 默认值 | 说明 |
|------|--------|------|
| max_stack | 1,048,576 | 单帧最大栈深度 |
| max_call_depth | 256 | 最大调用深度 |
| max_steps | 10,000,000 | 最大执行步数（0 = 无限制） |
| time_limit | 无限制 | 执行时间限制（秒） |

`#[stardust(id = N)]` 用于标注 `extern "stardust"` 函数的数字 ID；属性值必须是预期类型。未知属性或非法值会在类型检查阶段报错。Stardust 后端目前只接受这些属性，不生成运行时限制代码。

## 项目结构

```
freebird/
├── src/
│   ├── main.rs          # CLI 入口
│   ├── lib.rs           # 库入口
│   ├── frontend/        # 词法/语法分析
│   ├── hir/             # HIR + 名称解析
│   ├── typeck/          # 类型检查
│   ├── borrowck/        # 借用检查
│   ├── mir/             # MIR 生成
│   ├── vm/              # 虚拟机核心
│   ├── codegen/         # 代码生成
│   ├── repl.rs          # REPL
│   ├── io.rs            # IO 库
│   └── diagnostics.rs   # JSON 诊断
├── vscode/              # VSCode 扩展
├── readme.md            # 语言规范
├── USAGE.md             # 使用指南（本文件）
├── STATUS.md            # 规范章节 × 后端 × 测试的能力矩阵
├── ROADMAP.md           # 里程碑与验收标准
├── TASKS.md             # 已完成/待办事项
├── IMPROVEMENTS.md      # 历史审计快照
└── Cargo.toml
```
