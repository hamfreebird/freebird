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

### 编译

```bash
freebird build --target stardust hello.fb  # 编译为 Stardust 源码
```

`--target llvm` 尚未实现；请求该目标会直接报错退出。

## 当前实现状态

- VM 已支持数值/字符/布尔、变量与作用域、控制流、函数调用/递归、数组、结构体、枚举匹配、`Vec`、`Box`、字符串、元组解构以及 `read_*`/`print_*`/文件 I/O。
- 运行前会执行名称解析、基础类型检查和借用/移动检查；`check` 目前不报告借用错误。
- Stardust 后端可生成并运行上述大部分程序，复杂值统一放在 Stardust 全局堆上。`&mut` 引用使用堆单元间接访问，`*p = v` 会回写原变量（包括把引用传给函数后写入）。仍有限制：`file_read`/`file_write` 与远程栈只是占位；`read_line` 按 UTF-8 解码（上限 1024 个字符，非法首字节替换为 U+FFFD，不检测 EOF）。
- LLVM 后端尚未实现。

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
enum Option { None, Some(i32) }

fn map_opt(opt: Option) -> i32 {
    match opt {
        Option::None => 0,
        Option::Some(v) => v,
    }
}
```

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
```

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

## 运行时配置

通过 `#[key=value]` 注解覆盖运行时配置：

| 配置 | 默认值 | 说明 |
|------|--------|------|
| max_stack | 1,048,576 | 单帧最大栈深度 |
| max_call_depth | 256 | 最大调用深度 |
| max_steps | 10,000,000 | 最大执行步数 |

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
├── IMPROVEMENTS.md      # 实现计划
└── Cargo.toml
```
