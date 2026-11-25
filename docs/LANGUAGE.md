# Freebird 语言文档

> 适用版本：实现 0.1.0 / 规范 v0.5.0
> 最后更新：2026-11-25

本文面向 Freebird 使用者，覆盖已实现的语法、类型、运行时与工具链。目标规范以
[`../readme.md`](../readme.md) 为准，能力矩阵与测试覆盖见 [`../STATUS.md`](../STATUS.md)；
两者冲突时以 `src/` 与测试为准。维护规则见本文第 22 节。

## 1. 快速开始

构建工具链：

```bash
cargo build --release
./target/release/freebird --version   # 版本来自 Cargo.toml
```

最小程序 `hello.fb`：

```freebird
fn main() {
    print_str("Hello, Freebird!\n");
}
```

运行与检查：

```bash
freebird run hello.fb          # VM 后端（默认）
freebird check hello.fb        # 只做解析/名称/类型/借用检查
freebird build --target stardust hello.fb -o hello.sd
freebird build --target llvm --emit exe hello.fb -o hello
```

三个后端：

| 后端 | 执行方式 | 说明 |
|---|---|---|
| VM | `freebird run` | 默认后端；数值、容器、引用、I/O 等已覆盖 |
| Stardust | `build --target stardust` | 生成 Stardust 源码，交给 bundled 解释器执行 |
| LLVM | `build --target llvm` | 复用 Stardust 的 LLVM 编译器生成 IR 或可执行文件 |

多文件项目使用 `freebird.toml`（见第 14 节）。

## 2. 词法与源文件

- 源文件为 UTF-8 文本，扩展名 `.fb`。
- 注释：`// 单行`，`/* 块注释 */`。
- 标识符由字母、数字和下划线组成，不能以数字开头；`_` 单独出现是通配符。
- 关键字包括 `fn`、`let`、`mut`、`if`、`else`、`while`、`loop`、`for`、`in`、`match`、
  `return`、`ret`、`struct`、`enum`、`impl`、`trait`、`mod`、`pub`、`extern`、`use`、
  `unsafe`、`as`、`const`、`break`、`continue`、`true`、`false`、`stardust`。

字面量：

```freebird
let a = 42;          // 默认 i32
let b = 42i64;       // 整数后缀：i8/i16/i32/i64/isize/u8/u16/u32/u64/usize
let c = 3.5;         // 浮点，默认 f64
let d = true;
let e = 'x';         // char（Unicode 标量值）
let f = "text";      // String
let g = ();          // unit
```

运算符：`+ - * / %`、`== != < > <= >=`、`&& || ^`、`!`、`& &mut *`、`as`、`=`。
分隔符：`( ) [ ] { } , ; : :: . -> =>`。

## 3. 类型系统

标量类型：`i8`/`i16`/`i32`/`i64`/`isize`、`u8`/`u16`/`u32`/`u64`/`usize`、
`f32`/`f64`、`bool`、`char`、`()`。

复合与引用类型：

```freebird
(T1, T2)          // 元组
[T; N]            // 数组
Vec<T>            // 动态数组
String            // 字符串
Box<T>            // 单值堆盒
&T / &mut T       // 不可变/可变引用
fn(A, B) -> C     // 函数指针（多返回值写作 fn(A) -> (B, C)）
Point / Maybe<T>  // 结构体 / 枚举
Option<T> / Result<T, E>
```

- 类型可由标注、字段和实参推断；数值类型之间允许隐式赋值兼容，其余要求精确匹配
  （未知类型不做误报）。
- 泛型采用类型擦除布局 + 按实例注册/函数体单态化（见第 10 节）。
- 不支持 `dyn Trait`、关联类型与 trait 对象。

## 4. 变量、作用域与双栈模型

```freebird
let x = 1;            // 不可变
let mut y = 2;
y = 3;
{
    let x = "shadow";  // 块内遮蔽
}
```

- `let` 支持变量、元组与结构体解构；可失败模式（枚举变体、字面量）必须改用 `match`
  （违反报 T021）。
- 赋值目标：变量、`*p`、`p.field`、`arr[i]`；不可变变量赋值报 R006，非法目标报 R007。
- 借用/移动检查在编译期执行：移动后使用、可变借用冲突、借用期间移动等会报 B001–B006；
  `&mut` 的写入会跨调用帧回写到原变量。
- 运行时使用“ID 栈 + 值栈”的双栈模型；变量访问与 `roll` 的精确语义见规范 §3–§4。
  用户通常不需要直接操作它们，`unsafe` 下的值栈见第 15 节。
- 不支持可变参数（`fn f(mut x: T)` 不是语法的一部分）。

## 5. 表达式与运算符

Freebird 是表达式语言：`if`、`match`、`loop`、块都可以产生值。

```freebird
let n = if flag { 1 } else { 2 };
let m = match n { 0 => 10, _ => 20 };
let v = loop { break 7; };
let s = { let a = 1; a + 2 };
```

- 块的值是尾表达式（不带分号的最后一个表达式）；`return a, b;` 返回多个值。
- `ret n` 是低级的立即数返回元数指令，`n` 必须为整数字面量，且与函数签名一致。
- 调用：`f(a, b)`；方法：`x.method(...)`；关联函数：`Type::f(...)`；显式类型实参：`f::<T>(...)`。
- 字段/索引：`p.x`、`arr[i]`、`v[0]`；支持字段与索引赋值。
- 字面量：元组 `(a, b)`、数组 `[a, b]`、`vec![a, b]`、结构体 `Point { x: 1, y: 2 }`、
  枚举 `Maybe::Some(1)`。
- `expr as T` 支持标量数值、`char`、`bool` 之间的转换。

## 6. 控制流

```freebird
if cond { ... } else if other { ... } else { ... }

while cond { ... }

loop {
    if done { break; }
}

'outer: loop {
    loop {
        break 'outer;
    }
}

for item in items {
    if skip { continue; }
    print(item);
}
```

- `loop` 支持值语义：`let x = loop { break 42; };`。
- `break value` 只在 `loop` 中合法；`while`/`for` 中使用会报错。
- 循环标签 `'outer:` 与 `break 'outer`/`continue 'outer` 适用于 `loop`/`while`/`for`。
- `if let pat = expr { ... }` 与 `while let pat = expr { ... }` 在解析阶段降级为
  `match`/`loop`。
- `for` 可迭代数组、`Vec<T>`、`String`（产生 `char`）、`&Vec<T>`/`&String`。

## 7. 模式匹配

支持的模式：

```freebird
match value {
    0 => ...,
    n if n > 10 => ...,
    Point { x, y: 0 } => ...,
    Point { x, .. } => ...,
    Maybe::Some(inner) => ...,
    (a, b) => ...,
    &ref_value => ...,
    1 | 2 | 3 => ...,
    _ => ...,
}
```

- 支持字面量、变量、`_`、元组（含嵌套与 `..`）、结构体（字段简写与 `..`）、枚举
  （嵌套载荷）、引用 `&pat`/`&mut pat`、`or` 与守卫。
- 类型检查会拒绝非穷尽 `match`（T015）、未知变体/字段（T006/T007/T008）、
  `or` 分支绑定不一致（T022）；重复或不可达分支产生警告 W001。
- `let` 支持元组与结构体解构；枚举/字面量模式必须用 `match`。
- `match` 的被匹配表达式可以是调用、二元表达式或括号结构体字面量。

## 8. 函数、多值返回与函数指针

```freebird
fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn split(x: i64) -> (i64, i64) {
    return x / 2, x % 2;
}

fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}
```

- 尾表达式作为隐式返回值；`return` 可返回多个值。
- 递归与互递归可用；`&mut` 参数写入会回写调用方变量。
- 函数名可以作为值取得 `fn(...) -> ...`，支持零/单/多返回值的间接调用。
- 泛型函数见第 10 节；`impl` 方法见第 11 节；闭包见第 12 节。
- `extern "stardust"` 声明见第 15 节。

## 9. 结构体、枚举与容器

```freebird
struct Point { x: i64, y: i64 }
enum Shape { Dot, Rect(i64, i64) }

let p = Point { x: 1, y: 2 };
let s = Shape::Rect(4, 5);
```

- 结构体字段访问/赋值；枚举构造与 `match` 解构。
- 数组 `[T; N]` 与元组；`Vec<T>` 支持 `push`/`pop`/`len`/`is_empty`/`clear` 与索引；
  `String` 支持 `len`/`push`/`pop`/`is_empty`/`clear`，可通过 `for` 迭代字符；
  `Box<T>` 通过 `Box::new` 构造、`*box` 解引用。
- 容器在运行时以堆块表示（`[len, cap, elems...]`），VM/Stardust 共享同一逻辑布局。
  Vec 扩容/原地 push-pop 已实现；数组/结构体 COW 与嵌套块的旧块按保守策略不回收
  （见 `docs/adr/0002-heap-reclamation-deferred.md`）。
- 索引越界：VM 报运行期错误；Stardust 以除零陷阱非零退出；LLVM 继承 Stardust。

## 10. 泛型与约束

```freebird
struct Pair<T> { first: T, second: T }
enum Maybe<T> { None, Some(T) }

fn id<T>(x: T) -> T { x }
fn pick<T: Ord>(a: T, b: T) -> T { a }

let a: Pair<i64> = Pair { first: 1, second: 2 };
let b: Pair<String> = Pair { first: "x", second: "y" };
id::<i64>(3);
```

- 类型实参从字段访问/调用实参推断；也可用 turbofish 显式指定。
- 内建 trait 约束：`Ord`、`Eq`、`Display`，覆盖整数、`bool`、`char`、`String`；
  不满足报 T026。
- 用户 trait 也可作为约束：实例化点检查是否存在 `impl Trait for Type`，缺失报 T026。
- 实例会被收集：`Pair<i64>`/`Pair<String>` 拥有独立布局；泛型函数会为每个实例复制
  函数体（独立 FnId，如 `id<i64>`/`id<String>`）。
- 限制：泛型体仍只允许与具体类型无关的操作；无 `dyn Trait`/关联类型；无完整 trait
  约束求解（只检查 impl 是否存在）。

## 11. trait 与 impl

```freebird
trait Greet {
    fn greet(&self) -> i64;
}

struct P { x: i64 }

impl Greet for P {
    fn greet(&self) -> i64 { self.x }
}

impl P {
    fn new(x: i64) -> P { P { x: x } }
    fn bump(&mut self, by: i64) { self.x = self.x + by; }
}
```

- `trait` 声明方法签名（允许无函数体）；`impl Trait for Type` 提供实现。
- 方法名在编译期扁平化为 `Type::name`；`x.greet()` 通过现有方法解析静态分发，
  用户方法优先、内建方法兜底。
- 接收者支持 `self`、`&self`、`&mut self`；`self.field = v` 会写回调用方变量。
- 关联函数用 `Type::f(...)` 调用；`impl<T>` 支持类型参数（类型擦除 MVP）。
- 用户 trait 可作为泛型约束（见第 10 节）。
- 限制：不校验 trait 方法是否全部实现；无 `dyn Trait`/vtable、关联类型、默认方法。

## 12. 闭包

```freebird
let inc = |x: i64| x + 1;
let seven = || 7;
let f: fn(i64) -> i64 = |x: i64| x * 2;

print(inc(2));
print(f(3));
```

- 闭包参数必须带类型标注；语法为 `|params| expr` 或 `|| expr`。
- **无捕获闭包**在 HIR 降级期提升为顶层函数，表达式位置得到普通函数指针，
  可传参、返回、间接调用；VM/Stardust/LLVM 行为一致。
- **捕获外部变量**的闭包报 R009；环境捕获与 `fn(env, args)` 调用约定尚未实现。
- 闭包体是单个表达式；可使用块表达式。

## 13. 编译期 const

```freebird
const LIMIT: i64 = 40 + 2;
const ENABLED: bool = LIMIT > 10;

fn main() {
    print(LIMIT);
}
```

- 语法：`const NAME: Type = expr;`，类型标注可省略。
- 求值子集：字面量、此前声明的 const、一元/二元算术、比较与逻辑。
- 求值在 HIR 降级期完成，使用点替换为字面量，三后端一致。
- 非法常量报 R008：除零、前向引用、整数溢出、不支持的表达式（函数调用、循环、堆分配等）。
- 限制：`const fn`、分支/循环求值尚未实现。

## 14. 模块、项目与依赖

单文件模块：

```freebird
// util.fb
pub fn double(x: i64) -> i64 { x * 2 }

// main.fb
mod util;
use util::double;

fn main() { print(double(21)); }
```

- `mod name;` 加载同目录 `name.fb`；模块内顶层函数注册为 `name::f`。
- `use name::f;` 在名称解析阶段建立别名；模块项默认私有，需 `pub` 才能跨文件使用。
- 结构体/枚举在模块中同样加 `name::` 前缀，类型引用与字面量会同步重写。
- 循环依赖会在解析阶段报错。

项目（`freebird.toml`）：

```toml
[package]
name = "app"
entry = "src/main.fb"

[dependencies]
util = { path = "../util" }
```

- `run`/`check`/`build` 支持 `--manifest-path <file|dir>`；无文件参数时自动发现
  `./freebird.toml`。
- 依赖递归展开，以 `<dep>::` 前缀注册；`freebird lock` 生成 `freebird.lock`
  （相对路径 + 包内 `.fb` 内容哈希），构建时校验。
- 限制：仅路径依赖，无版本解析、registry 或传递版本约束。

## 15. unsafe、stardust! 与 FFI

`unsafe` 值栈：

```freebird
unsafe {
    push(1);
    push(2);
    swap();
    let x = pop();   // 2
    roll(1);
}
```

- 仅能在 `unsafe` 块内调用；否则类型检查拒绝（T018）。
- `push(x)`、`pop()`、`swap()`、`dup()`、`roll(n)` 操作独立值栈；`roll` 遵循规范 §4.2.3
  （正/零/负三向）。空栈 pop、元素不足的 swap/dup、越界 roll 报运行期错误。
- Stardust 侧值栈从 64 起按需翻倍扩容。

`stardust!` 宏：

```freebird
let v = stardust! { Push(5) Push(3) Add };
```

- 支持 Stardust 指令集（Push/Pop/Dup/Swap/算术/比较/逻辑/Store/Load/ShiftL/ShiftR/
  Depth/Pick/DropN/Reverse/Rotate/Mark/Jump/UncondJump/Call/Ret/NumOut/NumIn/CharOut/
  CharIn/DumpStack/DumpState/Breakpoint/Nop）；未知指令报 P014。
- 宏入口/出口会隐藏并恢复 Freebird 的 CallElement，使整栈指令只看到用户数据。
- `Call(id, m[, r])` 消费 `m` 个值；可选 `r` 声明 Freebird 栈模型应压入的返回值个数
  （默认 0）。VM 无法执行原始 `Call`（报 `RawCallUnsupported`）。

FFI：

```freebird
extern "stardust" {
    #[stardust(id = 7)]
    fn ext(x: i64) -> i64;
}

fn main() {
    unsafe {
        print(ext(1));
    }
}
```

- `extern` 声明的函数只能在 `unsafe` 块内调用（T017）；不能作为函数值。
- 省略 `id` 时按函数名做 SipHash-2-4 映射到 `1..=999`；重复/过大 ID 编译期报错。
- `build --target stardust|llvm --link-stardust <file>` 可追加外部 Stardust 定义。
- VM 执行 FFI 会报 `FfiUnsupported`；LLVM 目标需要外部定义参与链接。

后端限制：

- LLVM 目标继承 Stardust 的语义与限制；文件 I/O 由 `m3_test` 覆盖。
- Unicode `CharOut` 通过 `#[char_mode=unicode]` 支持。
- 运行时限制属性在 LLVM 目标上**编译期拒绝**（见第 16 节）。

## 16. 运行时限制

在 `main` 上声明：

```freebird
#[max_stack=1024]
#[max_call_depth=64]
#[max_steps=1000000]
#[time_limit=5]
fn main() {
    // ...
}
```

- VM 读取这些属性并应用到 `RuntimeConfig`；`time_limit` 以秒为单位。
- `build --target stardust` 会把它们写入生成源码顶部（如 `#[max_steps=1000000]`），
  由 Stardust 解释器在词法分析前读取并执行。
- LLVM 目标只转发 `char_mode`，不执行这些限制；Freebird 在 `--target llvm` 时
  编译期报错，避免误以为限制生效。
- `freebird run --debug` 使用 `RuntimeConfig::debug()`，与 CLI 默认值不同。

## 17. 标准库与内建函数

内建函数（`src/io.rs::BUILTINS`）：

| 函数 | 签名 | 说明 |
|---|---|---|
| `print` / `print_int` | `(i64) -> ()` | 输出整数并换行 |
| `print_char` | `(char) -> ()` | 输出字符 |
| `print_str` | `(String) -> ()` | 输出字符串（不换行） |
| `println_str` | `(String) -> ()` | 输出字符串并换行 |
| `read_int` | `() -> i64` | 读取整数 |
| `read_char` | `() -> char` | 读取字符 |
| `read_line` | `() -> String` | 读取一行（UTF-8 lossy；EOF 报 I/O 错误） |
| `file_read` / `file_write` | `(String) -> String` / `(String, String) -> ()` | UTF-8 文本文件 I/O |
| `abs` | `(i64) -> i64` | 绝对值 |
| `min` / `max` | `(i64, i64) -> i64` | 最小值/最大值 |
| `int_to_str` | `(i64) -> String` | 十进制转换 |
| `str_to_int` | `(String) -> Option<i64>` | 解析整数 |
| `format` | `(String, args...) -> String` | 占位符格式化（见下） |

`format`：

```freebird
let s: String = format("x={} ok={}", 42, true);
let escaped: String = format("{{literal}} {}", "v");
```

- `{}` 按顺序消费实参，`{{`/`}}` 输出字面花括号。
- 实参支持整数、`bool`、`char`、`String`。
- 字面量模板在编译期校验占位符数量（T025）；实参类型不支持报 T024；
  变量模板在运行期校验，VM/Stardust/LLVM 一致地报错。

`Option`/`Result`：

- `Option`：`is_some`、`is_none`、`unwrap`、`unwrap_or`、`ok_or`、`map`。
- `Result`：`is_ok`、`is_err`、`unwrap`、`unwrap_or`、`ok`、`err`、`map`。

`Vec`/`String`：`len`、`push`、`pop`、`is_empty`、`clear`。数组/元组支持索引与解构。
字符串在运行时是 Unicode 码点序列；Stardust 的 `read_line` 与 VM 的
`String::from_utf8_lossy` 行为对齐（非法字节 → U+FFFD）。

## 18. 工具链

| 命令 | 作用 |
|---|---|
| `freebird run <file\|--manifest-path>` | VM 执行，输出程序 stdout |
| `freebird check <file\|--manifest-path>` | 解析 → 名称解析 → 类型 → 借用检查，输出 JSON 诊断 |
| `freebird build --target stardust` | 生成 Stardust 源码（`-o` 指定输出） |
| `freebird build --target llvm` | 生成 LLVM IR 或可执行文件（`--emit ir\|exe`） |
| `freebird fmt` | 规范化缩进/空白（`--check`/`--write`/`--stdin`） |
| `freebird lsp` | stdio JSON-RPC：诊断、hover、定义、引用、重命名、符号、补全 |
| `freebird dap` | stdio DAP：断点、单步、栈、变量、pause |
| `freebird repl` | 交互式会话；`let` 绑定跨输入保留 |
| `freebird lock` | 生成/更新 `freebird.lock` |
| `freebird-bench` | 基准套件（`--check --tolerance`） |

构建选项：`--opt-level 0-3`、`--no-opt`、`--stardust-bin <path>`、
`--link-stardust <file>`。Stardust 二进制查找顺序：`--stardust-bin` →
`STARDUST_BIN` → `PATH` → bundled `stardust/stardust-linux-x86_64` → 兄弟仓库
`../esolang_stardust/target/{release,debug}`。LLVM 目标还需要 `clang`。

`check` 输出 JSON，含 `schema_version: 1`、稳定错误码、1-based span、可选
`suggestion`/`related`；项目模式诊断带 `file`（相对清单根）。

平台：当前只发布并支持 Linux x86_64（Stardust 工具链只有该平台构建）；VM 与工具链
代码可在 macOS/Windows 上编译（CI 做 `cargo check` 探测）。

## 19. 诊断与错误码

错误码格式：`P`/`R`/`T`/`B`/`W` + 三位数字。保持稳定；确需变更时在 CHANGELOG 说明。

### 解析/词法（P）

| 码 | 含义 |
|---|---|
| P001 | 未闭合字符串 |
| P002 | 未闭合字符 |
| P003 | 非法字符转义 |
| P004 | 非法转义序列 |
| P005 | 非法字符 |
| P006 | 非法数字 |
| P007 | 非预期 token |
| P008 | 非预期 EOF |
| P009 | 重复字段 |
| P010 | 未知属性 |
| P011 | 属性缺少参数 |
| P012 | 属性值非法 |
| P013 | `ret` 操作数非法 |
| P014 | 未知 `stardust!` 指令 |
| P015 | 位置/命名实参混用（解析层） |

### 名称解析（R）

| 码 | 含义 |
|---|---|
| R001 | 未定义变量 |
| R002 | 前向引用 |
| R003 | 重复定义 |
| R004 | 重复字段 |
| R005 | 参数数量不匹配 |
| R006 | 对不可变变量赋值 |
| R007 | 非法赋值目标 |
| R008 | 非法 `const`（求值失败/溢出/前向引用等） |
| R009 | 闭包捕获外部变量（暂不支持） |

### 类型检查（T）

| 码 | 含义 |
|---|---|
| T001 | 类型不匹配 |
| T002 | 未知类型 |
| T003 | 条件不是 `bool` |
| T004 | 表达式不可调用 |
| T005 | 未知函数 |
| T006 | 未知字段 |
| T007 | 未知变体 |
| T008 | 未知结构体 |
| T009 | 参数数量不匹配 |
| T010 | 返回数量不匹配 |
| T011 | 循环外 `break`/`continue` |
| T012 | 未知循环标签 |
| T013 | `break value` 位置非法 |
| T014 | `loop` 值位置非法 |
| T015 | 非穷尽 `match` |
| T016 | 普通函数缺少函数体 |
| T017 | extern 调用不在 `unsafe` 中 |
| T018 | 未支持特性（`#id`、闭包旧路径等） |
| T019 | 未知属性 |
| T020 | 非法属性 |
| T021 | `let` 使用可失败模式 |
| T022 | `or` 分支绑定不一致 |
| T023 | 泛型实参数量不匹配 |
| T024 | `format` 实参类型不支持 |
| T025 | `format` 模板占位符/转义非法 |
| T026 | 泛型实参不满足 trait 约束 |

### 借用检查（B）

| 码 | 含义 |
|---|---|
| B001 | 移动后使用 |
| B002 | 可变借用期间使用 |
| B003 | 可变借用期间借用 |
| B004 | 可变借用与不可变借用冲突 |
| B005 | 借用期间移动 |
| B006 | 对不可变变量赋值 |

### 警告（W）

| 码 | 含义 |
|---|---|
| W001 | 重复或不可达的 `match` 分支 |

## 20. 已知限制

以下限制与 `STATUS.md`/`ROADMAP.md` 同步维护：

1. **闭包捕获**：仅支持无捕获闭包；环境结构体与 `fn(env, args)` 调用约定未实现（R009）。
2. **trait 高级能力**：无 `dyn Trait`/vtable、关联类型、默认方法；不校验 trait 方法是否
   全部实现。
3. **泛型**：泛型体只能使用与具体类型无关的操作；无完整约束求解。
4. **const**：无 `const fn`、分支/循环求值。
5. **包管理**：仅路径依赖；无版本解析、registry 或传递版本约束。
6. **堆回收**：Stardust 对数组/结构体 COW 旧块与嵌套容器块保守不回收（ADR 0002）；
   VM 随值释放。
7. **LSP**：hover/completion 仍为文本级；definition/references/rename/documentSymbol
   基于 AST。
8. **LLVM**：不执行运行时限制属性（编译期拒绝）；继承 Stardust 的越界陷阱等行为。
9. **平台**：只发布 Linux x86_64；macOS/Windows 仅做编译探测。
10. **质量门禁**：ASan/UBSan 仍为过渡期告警；Miri 在 CI 阻塞运行，但本地工具链目录只读时
    无法安装组件。

## 21. 示例程序

斐波那契：

```freebird
fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn main() {
    for i in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
        print(fib(i));
    }
}
```

结构体、枚举与匹配：

```freebird
struct Point { x: i64, y: i64 }
enum Shape { Dot, Rect(i64, i64) }

fn area(s: Shape) -> i64 {
    match s {
        Shape::Dot => 0,
        Shape::Rect(w, h) => w * h,
    }
}

fn main() {
    let p = Point { x: 1, y: 2 };
    print(area(Shape::Rect(p.x, p.y)));
}
```

泛型、trait 与闭包：

```freebird
trait Show { fn show(&self) -> i64; }
struct P { x: i64 }
impl Show for P { fn show(&self) -> i64 { self.x } }

fn pick<T: Show>(v: T) -> i64 { v.show() }
fn twice(f: fn(i64) -> i64, x: i64) -> i64 { f(f(x)) }

fn main() {
    let p = P { x: 3 };
    print(pick(p));
    print(twice(|x: i64| x + 1, 5));
}
```

格式化与常量：

```freebird
const N: i64 = 3;

fn main() {
    print_str(format("n={} ok={}\n", N, true));
}
```

项目结构：

```text
app/
  freebird.toml
  src/main.fb
  src/util.fb
```

```toml
[package]
name = "app"
entry = "src/main.fb"
```

```freebird
// src/util.fb
pub fn double(x: i64) -> i64 { x * 2 }

// src/main.fb
mod util;
use util::double;
fn main() { print(double(21)); }
```

## 22. 文档同步规则

Freebird 的文档分为三层，变更必须同步对应层：

| 文档 | 职责 | 何时更新 |
|---|---|---|
| `readme.md` | 语言目标规范（权威语义） | 语言语义/类型/运行时模型变化 |
| `docs/LANGUAGE.md` | 用户语言文档（本文件） | 任何语言特性、语法、诊断码、限制变化 |
| `USAGE.md` | CLI 与工具链用法 | CLI 选项、命令、项目/REPL/LSP/DAP 行为变化 |
| `STATUS.md` | 规范 × 后端 × 测试矩阵 | 能力或测试覆盖变化 |
| `ROADMAP.md` | 里程碑与基线 | 里程碑完成/测试基线变化 |
| `docs/VERSIONING.md` | 版本号系统 | 版本策略/tag/bump 规则变化 |
| `docs/release.md` | 发布与 CI 流程 | 流水线、打包、发布步骤变化 |
| `CHANGELOG.md` | 逐版本变更记录 | 每个可发布变更 |
| `AGENTS.md` | 维护者命令/架构速览 | 命令、架构、关键约定变化 |

**每次变更的最低要求**：

1. 新增或修改语言行为：更新 `docs/LANGUAGE.md` 对应章节、`STATUS.md` 矩阵、
   `CHANGELOG.md`；新增测试（双后端一致时用 `assert_same_output`）。
2. 修改 CLI/工具链：更新 `USAGE.md` 与 `docs/LANGUAGE.md` 第 18 节。
3. 新增/修改诊断码：更新 `docs/LANGUAGE.md` 第 19 节、`STATUS.md`、诊断快照
   （`UPDATE_SNAPSHOTS=1 cargo test --test snapshot_test`）。
4. 版本/发布相关：更新 `docs/VERSIONING.md`、`docs/release.md` 与本文件、
   `VERSIONING.md` 的 `适用版本`/`最后更新` 头。
5. 每次提交前运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
python3 scripts/check-doc-links.py
python3 scripts/check-fixtures.py
python3 scripts/check-docs.py
python3 scripts/check-version.py
```

`docs/` 下的用户文档必须带有：

```text
> 适用版本：实现 <Cargo 版本> / 规范 v<规范版本>
> 最后更新：YYYY-MM-DD
```

`scripts/check-docs.py` 校验这些头、`docs/LANGUAGE.md` 的必填章节以及
`docs/README.md` 的索引完整性；CI 的 `docs` job 会阻塞不满足要求的提交。
