# ADR 0003：用户方法与 `impl` 是否纳入语言（M2-9）

- 状态：待决策（2026-11-23）
- 相关：`readme.md` §5/§11、`src/hir/types.rs`、`src/typeck/checker.rs`、`src/mir/generator.rs`、`ROADMAP.md` M2-9

## 背景

`readme.md` 的目标规范没有定义 `impl` 块、`self` 参数或用户定义方法。当前代码里
只有两条与“方法”相关的通路，且都绑定到内建类型：

- `HirExpr::MethodCall`（`x.push(1)`）：`TypeChecker::check_method_call` 按接收者的
  内建类型名（`Vec`/`String`/`Option`/`Result`）分派，未知方法报 `UnknownField`；
- `HirExpr::TypeMethod`（`Vec::new()`）：`check_type_method` 只识别
  `Vec::new`/`String::new`/`Box::new`。

两者在 MIR 里都直接展开为内建指令，**没有“用户类型 → 用户函数”的解析通路**，
parser 也没有 `impl` 关键字。因此 M2-9 首先是一个规范层决策：语言要不要有它。

## 方案

### 方案 A：不纳入语言（关闭 M2-9）

把 `impl`/方法从路线图移除，用户类型只能通过自由函数操作
（`fn area(r: Rect) -> i64`）。规范不变。

- 工作量：仅文档。
- 代价：`Option`/`Result`/`Vec` 有方法而用户类型没有，语言表面不一致；
  链式调用风格无法用于用户类型。

### 方案 B：仅关联函数

```freebird
impl Rect {
    fn square(side: i64) -> Rect { Rect { w: side, h: side } }
}

fn main() { let r = Rect::square(3); }
```

- 解析：`impl Type { fn ... }` 块，函数名扁平化为 `Type::name`。
- 名称解析/类型检查/MIR：与普通自由函数完全一致，只是名字带 `Type::` 前缀。
- Stardust：函数名本就扁平化，FnId dispatch 无需改动。
- 借用检查：不涉及 `self`，规则不变。
- 工作量：小（主要成本在 parser + AST/HIR + 名称冲突检测）。

### 方案 C：关联函数 + 方法（推荐）

在 B 的基础上支持 `self` 接收者：

```freebird
impl Rect {
    fn area(&self) -> i64 { self.w * self.h }
    fn grow(&mut self, by: i64) { self.w = self.w + by; }
}

fn main() {
    let mut r = Rect { w: 2, h: 3 };
    print(r.area());
    r.grow(1);
}
```

- 解析：`self` / `&self` / `&mut self` 作为首个参数；`self.w` 解析为接收者的字段访问。
- 名称解析：`Type::method` 进函数表；调用 `x.m(...)` 时由接收者类型解析到 `Rect::m`，
  解析失败才回落到内建方法分派。
- MIR：把 receiver 作为第 0 个实参压栈，复用既有调用指令；`&self`/`&mut self`
  与自由函数的 `&T`/`&mut T` 参数走同一条 `Ref`/`Deref` 通路。
- 借用检查：`&self` 等价于共享借用、`&mut self` 等价于独占借用，复用
  `BorrowChecker` 现有的引用参数规则（无需新概念）。
- Stardust：方法调用同样扁平化为普通函数调用，无后端特例。
- 工作量：中等（比 B 多出 `self` 参数绑定、方法解析顺序与借用检查接线）。

## 影响面

| 项目 | A | B | C |
|---|---|---|---|
| `src/frontend`（lexer/parser/AST） | — | `impl` 关键字与块 | 额外 `self` 参数语法 |
| `src/hir` | — | `ImplDecl` + 扁平化函数名 | 额外 `self` 绑定与方法调用解析 |
| `src/typeck` | — | 签名检查（同自由函数） | 方法解析顺序、`self` 类型 |
| `src/mir` | — | 无变化（名字扁平化） | receiver 作为首参压栈 |
| `src/codegen`（Stardust） | — | 无变化 | 无变化 |
| `src/borrowck` | — | 无变化 | `&self`/`&mut self` 复用引用规则 |
| 规范/文档 | 移除 M2-9 | 新增 §5 小节 | 新增 §5 小节 + 示例 |

## 决策

**采用方案 C（2026-11-23）**：语言纳入 `impl` 块、关联函数与 `self`/`&self`/`&mut self` 方法。

落地约定：

1. **语法**：`impl Type { fn f(self, ...) ... }`；`self` 是保留接收者名，可写 `self`、
   `&self`、`&mut self`。`self.field` 解析为接收者的字段访问。
2. **名字扁平化**：`impl` 内的函数在 HIR 中登记为 `Type::f`（与自由函数同一张表），
   冲突按“重复定义”报错。
3. **方法解析**：`x.m(args)` 先按接收者类型查 `Type::m`；命中则按普通函数调用检查
   （receiver 作为第 0 实参），未命中才回落到内建方法分派（`Vec`/`String`/`Option`/`Result`）。
4. **类型检查产出标注**：解析结果以 `Span → 函数名` 记录，随现有的方法标注表交给
   MIR 生成器（MIR 生成阶段没有类型信息）。
5. **MIR**：用户方法调用求值 receiver 与实参后，复用既有 `Call` 指令；`&self`/`&mut self`
   与自由函数引用参数走同一条 `Ref`/`Deref` 通路。
6. **Stardust**：函数名已扁平化，无需后端特例。
7. **借用检查**：`&self` = 共享借用、`&mut self` = 独占借用，复用 `BorrowChecker`
   既有引用参数规则。

### 分期落地（2026-11-23）

- **已实现**：`impl` 块、关联函数（`Rect::square(3)`）、值接收者方法（`fn area(self)`，
  含 `self.field`）。关联函数调用在降级阶段解析为普通函数调用；方法调用由类型检查
  解析为 `Type::name` 并把 receiver 作为第 0 个实参。
- **未实现**：`&self` / `&mut self` 接收者。当前由类型检查报
  `UnsupportedFeature`（而不是落到字段/类型错误的误导信息），后续实现时需要在
  调用点生成 `Ref` 并对方法体做自动解引用。

同步更新 `readme.md` §5、`STATUS.md`、`TASKS.md`、`USAGE.md`。

## 后果

- 选 A：M2-9 关闭，路线图减少一项；用户类型保持自由函数风格。
- 选 B：语言获得关联函数；方法调用仍只对内建类型可用。
- 选 C：语言表面与内建类型一致；代价是 parser/typeck/MIR 各多一条 `self` 通路，
  且需要在 `check_method_call` 中明确“用户方法优先、内建方法兜底”的解析顺序。
