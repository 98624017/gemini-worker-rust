# Code Quality Governance Design

日期：2026-04-26

## 背景

项目经过多轮迭代后，`rust-sync-proxy` 已从单一 Gemini 同步代理扩展为包含请求改写、响应物化、缓存、上传、OpenAI image 兼容、aiapidev 任务轮询和 admin 观测页面的服务。

当前主要问题不是单点功能缺陷，而是迭代累积后的重复逻辑、错误语义不一致、局部性能浪费和测试维护成本上升。

## 目标

本轮治理采用轻量综合方案：先设计，再分阶段实施，但不做大规模架构拆分。

目标包括：

1. 修正低风险但影响排障质量的行为不一致。
2. 降低请求/响应主链路中的重复解析、重复复制和统计漂移。
3. 补强关键边界测试，确保后续治理有回归保护。
4. 保持改动范围可控，避免一次性重排 `router.rs`、`admin.rs` 或 provider 边界。

## 非目标

本轮明确不做以下工作：

1. 不拆分 `src/http/router.rs` 为多个 provider/router 模块。
2. 不拆分 `src/admin.rs` 的 HTML、状态、认证、统计和脱敏逻辑。
3. 不把 OpenAI image、aiapidev、admin UI 迁移到全新架构。
4. 不引入新的大型依赖或重写 HTTP 转发模型。
5. 不改变已有公开 API、环境变量名称、默认路由和响应成功格式。

## 已确认的问题

### 行为一致性

- `model_action` 和 `image_generations_action` 有重复的请求元信息提取、读 body、JSON 解析和 admin 日志拼装。
- Gemini 路径会先在入口解析 JSON，再在 `forward_gemini_request` 中再次读取并解析，造成重复工作和计时叠加风险。
- 一部分代理错误响应包含 `source/stage/kind`，另一部分只有 `code/message`，不利于客户端和 admin 日志稳定归因。
- 无效 JSON、请求体读取失败等客户端侧错误目前容易被包装成 `502`，会误导调用方重试和告警。

### 局部性能与内存

- `keep_largest_inline_image` 会 clone 包含大 base64 的 `Value`。
- 响应侧 inlineData 扫描/patch 用完整 base64 作为 key，存在重复复制和 O(n²) 去重风险。
- PNG 压缩未启用、非 PNG、小于阈值或压缩无收益时仍可能复制整图。
- URL 输出模式下多个 inlineData 上传串行执行，多图场景延迟随图片数量线性增长。

### 测试与配置维护

- `Config` 默认值和测试构造存在重复，已有运行时默认值与测试默认值漂移。
- admin 鉴权、日志容量淘汰、去重和 UTF-8 截断边界测试不足。
- 测试中的 server 启动、admin auth header、配置构造 helper 存在重复。

## 推荐方案

采用三阶段治理，每阶段保持小步提交、测试通过后再进入下一阶段。

### 阶段一：行为一致性与低风险修复

优先处理会影响线上排障和客户端行为的问题。

范围：

1. 统一代理自身生成的错误响应结构，稳定包含 `source`、`stage`、`kind`。
2. 区分客户端输入错误和上游/代理错误。
   - 无效 JSON 返回 `400`。
   - 明确超过请求体限制时返回 `413`。
   - 上游连接、超时、响应读取失败继续归类为 `502`。
3. 消除 Gemini 标准链路的请求 JSON 二次解析。
4. 修正 admin 计时字段重复累加风险。
5. 为上述行为补回归测试。

约束：

- 可以在 `router.rs` 内部提取小 helper。
- 不移动 provider 逻辑到新文件。
- 不改变成功响应结构。

### 阶段二：局部性能与内存优化

只做低耦合、可单测验证的优化，不改变模块边界。

范围：

1. 将 `keep_largest_inline_image` 改为 move-based 或原地过滤，避免克隆大 base64。
2. 调整图片压缩返回语义，未变化时避免复制整图。
3. 优化 response inlineData 扫描/patch 的去重策略，避免完整 base64 作为重复 key。
4. 为 URL 输出多图上传并发化写实现计划，但实施需单独评估失败语义。

约束：

- 不优先改 R2 签名或 streaming 上传模型。
- 不改变 fail-open/fail-closed 策略，除非先在计划中明确兼容影响。

### 阶段三：测试与配置一致性

在不做架构拆分的前提下补强维护基础。

范围：

1. 收敛测试配置构造，减少与运行时默认值漂移。
2. 增加 admin Basic Auth 边界测试。
3. 增加 admin 日志容量、顺序、去重和 UTF-8 截断测试。
4. 抽取测试 helper 仅限 `tests/common`，不影响生产代码边界。

约束：

- 不重写 admin UI。
- 不把内嵌 HTML/JS 拆出为静态资源。

## 数据流设计

阶段一实施后，标准 Gemini 请求链路应保持以下责任分布：

```text
HTTP Request
  └── 入口解析：读取 body、解析 JSON、构造日志快照、解析上游
      └── forward_gemini_request：接收已解析 body 和原始 bytes
          ├── 请求图片 materialize
          ├── 请求 body encode
          ├── 发送上游请求
          ├── 响应 normalize/materialize
          └── 返回 Response + AdminLogEntry
```

关键点：

- JSON 解析只发生一次。
- 请求读取和解析错误由入口统一分类。
- 转发函数不再重复承担请求读取职责。
- admin 日志字段由单一层级负责累加，避免重复计时。

## 错误处理设计

代理自身生成的错误统一使用结构化格式：

```json
{
  "error": {
    "code": 400,
    "message": "invalid request json body",
    "source": "proxy",
    "stage": "parse_request_json",
    "kind": "invalid_json"
  }
}
```

分类规则：

- `400`：客户端输入无法解析或不满足代理校验。
- `413`：请求体超过代理限制。
- `502`：上游连接、超时、上游响应读取、上游 JSON 解析或代理处理上游数据失败。
- 上游非成功状态如果透传原响应，保持透传语义；如由代理包装，则必须带 `source/stage/kind`。

## 测试策略

每个阶段必须至少运行：

```bash
timeout 60s cargo test
```

阶段一重点测试：

- Gemini 路径无效 JSON 返回 `400`。
- OpenAI image 路径无效 JSON 返回 `400`。
- 请求体超限返回 `413`。
- 上游连接错误仍返回结构化 `502`。
- admin 日志里的错误字段与响应结构一致。
- Gemini 标准链路不会重复解析请求 JSON，相关计时不重复累加。

阶段二重点测试：

- 多 inlineData parts 下只保留最大图片且不改变非图片 part。
- 未启用压缩、非 PNG、小图、压缩无收益时响应内容不变。
- 大图路径减少不必要复制的行为通过单测或局部 benchmark 说明。

阶段三重点测试：

- 默认配置构造与运行时默认值一致。
- admin 鉴权边界覆盖错误密码、非法 base64、非 UTF-8、缺少冒号、未启用 admin。
- admin 日志容量保持 100 条，顺序正确，旧条目淘汰。
- URL 去重和 UTF-8 截断稳定。

## 风险控制

1. 每阶段只处理一类问题，避免行为修复和性能优化混杂。
2. 先补能暴露当前问题的测试，再修改实现。
3. 不进行大规模文件移动，降低合并冲突和回归风险。
4. 对错误码变化保持明确记录，因为这会影响客户端重试逻辑。
5. 对上传失败语义保持谨慎，URL 输出并发化必须先明确部分失败时的响应策略。

## 验收标准

本轮治理完成时应满足：

1. 设计范围内的行为修复都有回归测试。
2. `timeout 60s cargo test` 通过。
3. 代理自身错误响应结构一致。
4. Gemini 标准链路不再重复解析请求 JSON。
5. 第一批局部内存优化不改变成功响应语义。
6. 没有进行 `router.rs` / `admin.rs` 大拆分。

## 后续路线

如果本轮治理完成并稳定，可以在下一轮单独评估架构拆分，包括：

- 抽 `RequestContext` / `ParsedProxyRequest`。
- 抽 `AiApiDevClient`。
- 收敛 `openai_image` 模块职责。
- 拆分 admin 静态资源和运行时逻辑。

这些属于后续 C 类工作，不纳入本轮实施。
