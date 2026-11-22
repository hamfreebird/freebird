# ADR 0001：远程栈采用“逻辑远程栈 + 全局堆块”模型

- 状态：已采纳（2026-11-22，M1-4）
- 相关：`readme.md` §8、`src/codegen/stardust.rs`、`src/vm/mir.rs`

## 背景

`readme.md` §8 将 `Vec<T>` / `String` / `Box<T>` 描述为“本地栈保存 `StackId` 句柄，
指向独立的远程数据栈”。VM 曾按此模型实现 `AllocRemote` / `RemotePush` /
`RemotePop` / `RemoteLoad` / `RemoteStore` / `RemoteLen` / `SwitchToRemote` /
`SwitchToLocal` 等 MIR 指令。

但实际约束是：

1. 目标后端是仓库自带的 Stardust 解释器（`stardust/stardust-linux-x86_64`），
   它只有单层值栈和全局堆（`Store` / `Load`），**没有远程栈原语**。
2. MIR 生成器从未从任何源语言语法生成上述远程栈指令；`Vec` / `String` / `Box` /
   数组 / 结构体 / 枚举在 MIR 中统一是单槽句柄。Stardust 后端一直用全局堆块
   `[len, cap, elems...]` 模拟这些容器。
3. 这些指令在 Stardust 后端只能生成占位注释，违反“占位不是完成”的原则；
   同时 `stardust!` 宏的 `Store` / `Load` 被错误地映射到远程栈，而规范附录 A.2
   明确它们是**堆操作**。

## 决策

采用“逻辑远程栈 + 全局堆块”模型：

- 语言层面仍可称 `Vec` / `String` / `Box` 的底层存储为“远程数据”，但其运行时表示
  是**全局堆上的一个块**：`[length, capacity, elem_0, ...]`，本地栈只保存块起始地址
  作为句柄。
- 删除 MIR 中不可达的远程栈指令（`AllocRemote`、`FreeRemote`、`RemotePush`、
  `RemotePop`、`RemoteLoad`、`RemoteStore`、`RemoteLen`、`SwitchToRemote`、
  `SwitchToLocal`）及其 VM 实现。
- `stardust!` 的 `store` / `load` 映射为新增的 `HeapStore` / `HeapLoad`，语义与
  Stardust 原生 `Store` / `Load` 一致：`Store` 弹出地址再弹出值写入堆，`Load`
  弹出地址后压入 `heap[addr]`（未写入为 0）。VM 增加同名全局堆以保持两后端一致。
- 规范同步：§8、§8.5、附录 B 不再承诺“独立远程数据栈”这一实现细节。

## 后果

- Stardust 后端不再生成远程栈占位代码；`Vec` / `String` / `Box` 的 VM/Stardust
  差分测试保持逐字节一致。
- 堆块目前**只增不减**（`DropVar` 在 Stardust 后端仍是空操作）。安全回收需要
  引用计数或所有权跟踪，因为值语义的 copy-on-write 会让多个变量共享同一块；
  这是 M1-3 的遗留设计问题，单独处理。
- 仓库不再有“独立远程栈”的实现路径；`StackHandle` 仅保留为类型系统中的内部
  move 类型标记，不再有 MIR 指令产生它。
