# jemalloc 默认分配器设计

日期: 2026-04-10
状态: 已批准

## 目标

把 `rust-sync-proxy` 在 Linux GNU 环境下的默认全局分配器切换为
`jemalloc`，优先改善大流量突刺后的 RSS 回落能力。

本次设计目标：

1. 默认生产构建直接使用 `jemalloc`
2. 不把 allocator 参数硬编码进业务逻辑
3. 通过环境变量统一配置 `jemalloc` 的 purge / decay 策略
4. 保持非 Linux GNU 环境可继续构建与开发

## 非目标

本次不做以下事项：

- 不引入 `mimalloc`
- 不保留 feature 开关在 `jemalloc` 和系统 allocator 间切换
- 不加入 `malloc_trim` 或 glibc 专用兜底逻辑
- 不在 admin 面板暴露 allocator 统计
- 不在这一轮调 `narenas`

## 方案选择

本次评估了三条路径：

1. 代码内默认接入 `jemalloc`
2. 默认 `jemalloc`，但保留 feature 回退系统 allocator
3. 完全依赖 `LD_PRELOAD` 等部署侧注入

最终选择方案 1。

原因：

- 目标是“效果优先”，不需要为了回退路径保留额外复杂度
- 代码内接入比部署注入更稳定，构建、测试、容器行为一致
- 后续若要补 allocator 观测或 purge 控制，也更容易围绕统一实现扩展

## 接入策略

### 全局 allocator

新增独立的 `allocator` 模块：

- 在 `Linux + GNU` 环境下使用 `tikv-jemallocator`
- 其他平台保持系统 allocator

这样可以确保：

- 主二进制和测试共享相同 allocator 选择
- Docker 运行产物与本地 `cargo test` 行为一致
- 非目标平台不会因为 jemalloc 平台细节阻塞构建

### 运行时参数

不在 Rust 代码里解析或拼装 `MALLOC_CONF`。

统一约定：

- Docker 镜像给出默认 `MALLOC_CONF`
- 使用者可通过环境变量覆盖
- README 明确说明默认值和覆盖方法

批准后的默认值为：

```bash
MALLOC_CONF=background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500
```

参数含义：

- `background_thread:true`
  让 jemalloc 异步做回收工作，减少请求路径上的 purge 压力
- `dirty_decay_ms:500`
  允许短时间复用刚释放的页，但不长时间拖住 RSS
- `muzzy_decay_ms:500`
  保持与 dirty 页一致的回收节奏，先用均衡默认值

## 文件级改动

预计涉及：

- `Cargo.toml`
  增加 `tikv-jemallocator`
- `src/allocator.rs`
  承载全局 allocator 与默认推荐配置常量
- `src/lib.rs`
  暴露 allocator 信息给测试或未来诊断使用
- `src/main.rs`
  启动日志补充 allocator 名称
- `tests/allocator_test.rs`
  锁定默认 allocator 选择和推荐配置
- `Dockerfile`
  注入默认 `MALLOC_CONF`
- `README.md`
  补充 allocator 默认行为和调参说明

## 验证标准

### 构建验证

- `cargo test`
- `cargo build --release --locked`
- `docker build`

### 接入验证

- 启动二进制成功
- Docker 默认环境下进程能正常启动
- 构建产物可证明已包含 `jemalloc` 依赖

### 效果验证

至少保留一轮基础证据：

- 空载 RSS 基线
- 突刺后 RSS 回落观察

本轮重点不是一次性把参数调到最优，而是先把：

- allocator 切换
- 默认 decay
- 文档和构建链路

全部统一到位。
