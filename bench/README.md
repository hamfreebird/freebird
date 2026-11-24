# 基准套件与噪声基线（M5-1 / M6-4.4）

`bench/baseline.json` 记录 `freebird-bench` 的测量值（毫秒 / 字节），CI 的 `bench` job
用 `--check --tolerance 2.5` 做**非阻塞告警**（见 `docs/release.md` §7）。

## 测量方式

- 运行 `cargo run --release --bin freebird-bench`（`--no-llvm` 可跳过 LLVM 项）。
- 计时项由 `measure_ms` 采样 **3 次取中位数**，降低单次抖动。
- 字节项（Stardust 源码 / LLVM IR 体积）是确定值，不随机器变化。

## 噪声基线

同一台机器（Linux x86_64，release 构建）连续运行 `--check --tolerance 2.5`：

```
2026-11-25  运行 1：OK
2026-11-25  运行 2：OK
2026-11-25  运行 3：OK
```

结论：在当前基线与 `2.5x` 阈值下，本机连续运行可稳定复现（无回归告警）。
若某次出现回归，先按下面的方式排除机器噪声，再考虑更新基线。

## 阈值策略

| 场景 | 命令 | 说明 |
|---|---|---|
| 本地快速对比 | `cargo run --release --bin freebird-bench -- --check` | 默认 `1.5x`，用于发现明显退化 |
| CI 告警 | `cargo run --release --bin freebird-bench -- --check --tolerance 2.5` | 2.5x 吸收共享 runner 噪声；`continue-on-error`，不阻塞合并 |
| 更新基线 | `cargo run --release --bin freebird-bench -- --update` | 在无噪声、稳定机器上跑；提交 `bench/baseline.json` 的变更 |

## 复现与排查

1. 用 release 构建（debug 数值没有可比性）。
2. 关闭其他占用 CPU 的任务；连续跑 2–3 次确认是否稳定复现。
3. 计时项波动大时，可只跑 `--no-llvm` 缩小范围（LLVM 项会调用外部 Stardust 编译器）。
4. 确认是真实退化后再 `--update`，并在提交信息中写明变化原因。

bench 目前**不作为合并门禁**：基线只服务于趋势观察与发布前检查。
