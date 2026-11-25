# Changelog

本文件记录 Freebird 实现（Cargo 包版本）的显著变更。语言规范版本见 `readme.md` 顶部；
两者独立演进（当前规范 v0.5.0、实现 0.1.0）。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本遵循语义化版本。
0.x 阶段 minor 版本允许包含破坏性变更，但必须在对应条目中写明迁移方式（见 `docs/release.md`）。

## [Unreleased]

### Added

- **完整 CI 流水线**：阻塞门禁 `quality`（fmt/clippy/release build/全量测试/examples）、
  `docs`（链接/fixture/文档/版本/占位符）、`differential`、`llvm`、`miri`、`package`
  （发布 tarball + checksums）；`asan`/`ubsan`/`bench`/`cross-platform` 为告警，
  `fuzz` 在定时/手动触发。加入 concurrency、依赖缓存、超时与 artifact。
- **版本号系统**：新增 `docs/VERSIONING.md`（实现版本/规范版本双线、SemVer、预发布通道、
  bump 规则、兼容性承诺）与 `scripts/check-version.py`；CLI 版本改用
  `env!("CARGO_PKG_VERSION")`，与 `Cargo.toml` 单一来源一致；Release workflow 校验
  tag/Cargo/CHANGELOG/规范版本并自动提取 Release 说明。
- **完整语言文档**：新增 `docs/LANGUAGE.md`（22 章：语法、类型、匹配、泛型、trait、
  闭包、const、模块、unsafe/FFI、限制、错误码、示例、文档同步规则）与 `docs/README.md`
  索引；`docs/` 用户文档统一 `适用版本`/`最后更新` 头；`check-docs.py` 校验版本头、
  LANGUAGE 必填章节与索引；新增 `.github/PULL_REQUEST_TEMPLATE.md` 文档同步清单。
- **发布打包**：`scripts/package-release.sh` 统一生成 tar.gz、`.sha256` 与 `SHA256SUMS`
  （含二进制、readme、USAGE、CHANGELOG、LICENSE、docs、RELEASE-MANIFEST）；
  `scripts/changelog-section.py` 提取版本说明。
- **M5 质量基线**：
  - `freebird-bench` 基准套件与 `bench/baseline.json`，支持 `--check`/`--update`/`--tolerance`；
    CI 以非阻塞 job 运行。
  - 固定 seed 的随机程序差分 fuzz 与坏输入防 panic 测试（`tests/m5_test.rs`），失败自动最小化并归档。
  - 内存与 UB 审计记录（`docs/audits/0001-memory-ub-audit.md`）。
  - fixture 命名/stdin 配对检查（`scripts/check-fixtures.py`）。
  - 诊断快照（`tests/snapshots/diagnostics.txt`）与 `UPDATE_SNAPSHOTS=1` 更新流程。
  - 发布流程文档与本 CHANGELOG；`scripts/check-docs.py` 检查基线与版本一致性。
- **MIR 优化**：常量条件跳转折叠、无用跳转消除、Div/Mod/比较/逻辑常量折叠；
  `build --no-opt` 可关闭优化；`Line` 调试标记保留。
- **M6 包管理**：依赖递归展开（传递依赖同样以包名为模块前缀注册，同名包冲突与循环依赖在解析期报错）；
  `freebird lock` 生成 `freebird.lock`（依赖相对路径 + 包内 `.fb` 内容哈希），
  `run`/`check`/`build` 在锁文件存在时校验，缺失/路径变化/哈希不一致都会报错。
- **CI Miri job**：nightly 上运行核心模块（`vm`/`mir`/`borrowck`）的 `cargo miri test`，
  过渡期 `continue-on-error`；门禁策略见 `docs/release.md` §7。
- **M6-3.2 DAP**：运行中 `pause`（stdin 独立线程 + VM 分片执行，在分片边界停下）、
  `variables` 展开数组/结构体/枚举载荷/Vec/`Box`（惰性子项、可继续展开）、
  `stackTrace` 每帧返回自己的行号。
- **M6-3.4 REPL**：`let`/`let mut` 绑定跨输入保留（每次输入重放会话，使用全新 VM），
  `:vars` 列出会话绑定、`:clear` 一并清理；历史持久化到状态目录（`FREEBIRD_REPL_HISTORY`
  → `$XDG_STATE_HOME` → `~/.local/state`），多行输入压成一条记录，保留最近 200 条。
- **M6-3.5 `fmt`**：在行级规范化之外新增 token 级行内空白整理（`,`、`:`、`(`/`[`、`)`/`]`/`;`、
  `{`/`}` 的间距）与 `}`/`else` 换行合并；注释与字符串原样保留、不重排 token，
  `examples/` 全量格式化后仍可解析且幂等。
- **M6-4.2 持续 fuzz**：`tests/fuzz/corpus/` 内的固定语料（深层递归、长循环、嵌套容器、
  枚举 match、深表达式）在 VM/Stardust 两端差分；`FREEBIRD_FUZZ_SCALE` 控制随机用例数量与
  表达式深度；CI 新增每周定时 `fuzz` job（手动可触发），失败上传最小化后的归档产物。
- **M6-4.3 MIR 内联**：跨函数识别「零参数、返回单个常量」的叶子函数，在调用点把
  `CallElement + Call` 替换为常量 `Push`（保留 `Line`）；`--no-opt` 可关闭，
  `tests/m5_test.rs` 验证调用被消除且 VM/Stardust 输出不变。
- **M6-4.4 bench 噪声基线**：新增 `bench/README.md`，记录测量方式（计时项三次取中位数）、
  本机连续 `--check --tolerance 2.5` 的噪声基线、阈值策略（本地 1.5x / CI 2.5x）与复现步骤；
  bench 仍是非阻塞告警。
- **M6-4.5 跨平台**：明确只发布并支持 Linux x86_64（Stardust 工具链目前只有 Linux 构建）；
  CI 新增 `cross-platform` job，在 macOS/Windows 上做 `cargo check --all-targets` 探测（非阻塞）；
  平台边界与后续工作写入 `docs/release.md` §8。
- **M6-2.5 堆回收上限**：明确维持 Stardust 的保守回收策略，并测量 COW 写入的堆增长
  （约 10 个堆元素/次，1M 上限 ≈ 10 万次写入）；数据与结论写入 ADR 0002，
  `tests/m6_test.rs` 增加 1000 次 COW 压力的双后端一致性护栏。
- **CI 门禁收紧**：`miri` job 从 `continue-on-error` 转为阻塞；新增 `asan`/`ubsan`
  过渡期告警 job（nightly + `rust-src` + `-Zbuild-std`，分别跑 `address`/`undefined`
  sanitizer 的核心模块测试）。本地工具链目录只读，无法安装 `miri`/`rust-src`，
  实际验证依赖 CI；策略见 `docs/release.md` §7。
- **Stardust 运行时限制注解**：`main` 上的 `#[max_stack]`/`#[max_call_depth]`/`#[max_steps]`/
  `#[time_limit]` 现在会写入生成源码顶部（如 `#[max_steps=1000]`），由 Stardust 解释器在
  词法分析前读取并执行；VM 与 Stardust 均生效（`m6_test`）。LLVM 路径未验证。
- **LSP references/rename 语义化**：references/rename 改为基于 AST 的变量绑定与函数名位置
  （注释/字符串中的同名文本不再误报），解析失败时回退到原有文本匹配；
  hover/definition/documentSymbol/completion 仍为文本级。
- **M6-5.3 trait 静态分发**：新增 `trait Name { fn ...; }` 声明与 `impl Trait for Type { ... }`。
  impl 方法沿用 `Type::name` 扁平化，调用点复用现有用户方法解析（静态分发）；暂不做
  trait 约束检查、`dyn Trait`/vtable 或关联类型。`m6_test` 覆盖双后端输出。
- **LSP definition/documentSymbol 语义化**：definition 与 documentSymbol 改为基于 AST
  声明（函数/结构体/枚举/trait/const/let），注释中的假声明不再产生结果；
  hover/completion 仍为文本级。
- **泛型函数按实例单态化**：MIR 为每个实例预分配 FnId 并复制泛型函数体
  （`id<i64>`/`id<String>` 独立），调用点按类型检查注解解析到实例；`fn_info` 由 MIR
  生成器统一提供。泛型体仍只允许类型无关操作。
- **用户 trait 约束**：泛型参数可使用用户 trait 作为约束（`<T: Greet>`）；类型检查在
  结构体/枚举字面量与泛型函数调用点检查实参类型是否有对应 `impl Trait for Type`，缺失报
  T026。仍不支持 `dyn Trait`/vtable、关联类型与默认方法。
- **LLVM 运行时限制明确拒绝**：Stardust 的 LLVM 编译路径只转发 `char_mode`，不会执行
  `#[max_steps]` 等限制；Freebird 的 `--target llvm` 现在在编译期返回明确错误，避免
  用户误以为限制生效。VM 与 `--target stardust` 仍正常执行这些属性。
- **M6-5.3 无捕获闭包**：`|params| body` / `|| body` 现在会在 HIR 降级期提升为顶层函数，
  表达式位置得到函数指针，可传参、返回与间接调用（VM/Stardust/LLVM 一致）；捕获外部变量的
  闭包报 R009，环境捕获与 `fn(env, args)` 调用约定留待后续轮次。
- **M6-5.3 `const` 编译期求值**：新增顶层 `const NAME: Type = expr;`（类型标注可省略）。
  求值子集覆盖字面量、此前的 const、一元/二元算术/比较/逻辑；除零、前向引用、溢出与
  不支持的表达式报 R008。求值在 HIR 降级期完成，使用点直接替换为字面量，VM/Stardust/LLVM
  行为一致。闭包与用户自定义 trait 仍待排期。
- **M6-5.1 泛型约束与按实例布局**：支持内建 trait 约束 `<T: Ord + Eq + Display>`（覆盖整数、
  `bool`、`char`、`String`），在结构体/枚举字面量与泛型函数调用点校验，不满足报 T026。
  类型检查收集泛型实例，`TypeTable` 按实例注册布局（`Pair<i64>`/`Pair<String>` 字段类型独立替换），
  MIR 在构造点按实例查表并回退基名；泛型函数体仍按基名共用一份（类型擦除）。
- **M6-5.2 字符串格式化**：新增变参内建 `format(template, args...)` 与 `MirInst::Format`；
  模板中的 `{}` 依次消费实参，`{{`/`}}` 输出字面花括号，实参支持整数、`bool`、`char`、`String`。
  字面量模板在编译期校验占位符数量（T025），实参类型不受支持报 T024；变量模板在运行期校验，
  VM/Stardust/LLVM 一致报错。Stardust 侧先按实参种类构造字符串块数组，再对模板做
  “统计长度 + 写入字符”两遍扫描，输出 `[len, cap, chars...]` 堆块；LLVM 经 Stardust 继承。
- **M6-5 语言演进 ADR**：新增 `docs/adr/0006-language-evolution.md`，固定三条演进线
  （格式化 API → 泛型约束/按实例单态化 → 闭包/`const`/trait）的语法、实现方案、
  兼容策略（默认保持类型擦除、按需单态化）与分阶段验收；实现随后续里程碑排期。

### Fixed

- MIR 不再对未知表达式生成 `TODO` 注释；`#id` 统一由类型检查报 T018。
- 多文件项目 `check` 的诊断携带 `file`（相对清单根）与文件内行号。
- `stardust!` 的 `Call(n, m[, r])` 支持显式返回 arity（ADR 0005）；默认为 0。
- LLVM 后端支持 `#[char_mode=unicode]`，`CharOut` 按 UTF-8 输出（bundled Stardust 同步更新）。
- `build --link-stardust <file>` 可追加外部 Stardust 定义（FFI）；extern 函数作为值在编译期拒绝。
- LSP 新增 references/rename，位置按 UTF-16 处理；非 BMP 字符位置正确。
- LLVM 目标支持 `file_read`/`file_write`（Stardust LLVM 运行时按 UTF-8 编解码，bundled Stardust 同步更新）。
- 语句 span 改用语句起始 token：此前 `parse_block` 在消费 `;` 之后才取 `current_span()`，
  把语句行号算到下一行，导致调试断点不命中、诊断位置偏移。
- 优化期常量折叠改用 `checked_*`，避免优化器自身在溢出时 panic。

### Changed

- CI `test` job 更名 `quality`，新增 `docs` 与 `package` job；`release.yml` 拆分为
  verify/test/release 三阶段；发布产物由 `scripts/package-release.sh` 生成
  （tar.gz + sha256 + SHA256SUMS，替代单个 `freebird-linux-x86_64`）。
- 编译流水线新增 `CompileOptions` 与 `compile_*_with_options` 入口；
  `build` 的 Stardust/LLVM 生成默认开启优化。
- `read_line` 在 EOF 时统一报 I/O 错误（此前 VM 返回空字符串），与 Stardust/LLVM 一致；
  依赖空串判断输入结束的程序需要改为处理 I/O 错误。

## [0.1.0] - 2026-11-24

### Added

- **M0–M2 语言核心**：变量/作用域、控制流、模式匹配、`loop` 值语义与标签、函数/递归/多值返回、
  函数指针、泛型声明（类型擦除 MVP）、`Option`/`Result`、模块/`use`/`pub`、`impl` 方法、
  `&`/`&mut` 与借用检查、`unsafe` 值栈、`stardust!` 宏。
- **M3 后端与 FFI**：LLVM 后端（复用 Stardust 的 LLVM 编译器）、`extern "stardust"` FFI
  （SipHash-2-4 默认 ID、函数名冲突规避）、`stardust!` 指令补全与语义对齐。
- **M4 工具链**：`freebird fmt`、`freebird lsp`（诊断/hover/定义/符号/补全）、
  `freebird dap`（断点/单步/栈/变量）、REPL 增强、`freebird.toml` 路径依赖项目、
  诊断 `schema_version` 与稳定错误码。
- **工程**：CI 的 fmt/clippy、examples、文档链接、差分与 LLVM job；release workflow。

### Notes

- LLVM 目标不支持文件 I/O，非 ASCII `CharOut` 受 Stardust LLVM 的 ASCII 限制。
- 包管理仅支持路径依赖，无版本解析/registry/锁定文件。
