# 上游违规错误短期拦截缓存设计

日期：2026-04-30
状态：已确认，待实现

## 背景

下游调用方可能对失败请求有自动重试机制。当用户请求体包含违规提示词或不安全图片生成意图时，上游会返回明确的安全拦截错误，例如：

- `502 content blocked: {"error_code":"image_unsafe","message":"The generated images appear to be unsafe. Try modifying the prompts or the seeds."}`
- `400 Upstream moderation triggered: output_moderation`

这些错误通常不是瞬时故障，短时间内重复请求同一内容大概率仍会被拒绝。当前代理会把每次重试都继续转发给上游，造成上游请求、轮询和代理处理资源浪费。

## 目标

新增一个轻量的进程内短期缓存，用于记住已经被上游明确判定为违规的请求。

目标包括：

1. 相同违规请求在缓存有效期内直接返回上次的错误响应，不再转发上游。
2. 以上游真实返回为权威依据，代理不在首次请求前自行判断提示词违规。
3. 默认缓存时间为 5 分钟，可通过配置调整或关闭。
4. 缓存 key 不绑定 API key，避免同一违规请求换 key 后继续消耗上游资源。
5. 保持非违规错误和成功响应的现有语义不变。

## 非目标

本设计不做以下工作：

1. 不实现前置内容审核或提示词关键词拦截。
2. 不引入跨进程、跨实例共享缓存。
3. 不持久化到磁盘，进程重启后缓存可以丢失。
4. 不改变成功响应格式。
5. 不改变下游收到的违规错误 status、body 和 content-type。
6. 不把普通网络错误、限流错误或上游临时故障纳入违规缓存。

## 已确认决策

### 1. 缓存 key 粒度

缓存 key 使用：

```text
请求路径 + 上游 base_url + 规范化请求体 SHA-256
```

说明：

- 请求路径区分 Gemini 与 OpenAI image 等不同 API。
- 上游 base_url 区分不同上游服务。
- 请求体先解析为 JSON，再递归排序所有 object key，序列化为 canonical JSON bytes，最后计算 SHA-256。
- key 不包含 API key，避免同一违规请求通过更换 key 绕过短期拦截。

### 2. 默认缓存时间

默认 TTL 为 5 分钟：

```text
UPSTREAM_BLOCK_CACHE_TTL_MS=300000
```

当 TTL 配置为 `0` 时，该机制关闭。

### 3. 缓存容量

新增最大条目数配置：

```text
UPSTREAM_BLOCK_CACHE_MAX_ENTRIES=1024
```

达到上限后按 LRU 淘汰旧条目，避免大量不同违规请求撑爆内存。

## 推荐方案

采用“进程内 LRU + TTL 缓存”。

原因：

1. 当前问题是短时间重复请求造成的资源浪费，5 分钟内存缓存已能覆盖主要场景。
2. 首次请求仍交给上游判定，误伤风险低。
3. 改动范围小，适合插入现有 `router.rs` 的请求转发路径。
4. 不增加磁盘 IO、清理任务或外部服务依赖。

## 组件设计

新增模块建议命名为 `upstream_block_cache.rs`，职责保持单一。

### `UpstreamBlockCache`

负责：

1. 根据 key 查询缓存条目。
2. 写入可缓存的违规错误响应。
3. 按 TTL 过滤过期条目。
4. 按最大条目数淘汰旧条目。

缓存条目包含：

```text
status_code
content_type
body_bytes
expires_at
reason
```

其中 `reason` 只用于日志、测试或后续观测，不影响下游响应。

### `UpstreamBlockCacheKey`

负责生成稳定 key，输入包括：

```text
request_path
upstream_base_url
request_body_json
```

实现上可用 `sha2::Sha256` 生成请求体摘要，再拼接路径和上游地址。

请求体摘要必须基于 canonical JSON：

1. 忽略原始请求体空白、缩进和字段输入顺序。
2. 递归排序所有 JSON object key。
3. 保留数组顺序，因为数组顺序通常具有语义。
4. 不改写字符串内容、数字值、布尔值或 null。

### 违规错误分类 helper

新增一个纯函数判断错误响应是否可缓存。

首版匹配规则：

1. HTTP status 必须是 `400` 或 `502`。
2. body 文本大小写不敏感匹配以下关键词：
   - `content blocked`
   - `image_unsafe`
   - `upstream moderation triggered`

同时满足 status 与关键词条件，才认为这是可缓存的上游违规错误。

## 数据流设计

标准请求流程调整为：

```text
HTTP Request
  └── 读取 body 并解析 JSON
      └── resolve upstream
          └── 生成 block-cache key
              ├── 命中缓存：直接返回缓存的错误响应
              └── 未命中：继续走现有上游转发流程
                    └── 收到上游错误响应
                        ├── 命中违规关键词：写入缓存，然后返回该错误
                        └── 未命中：按现有逻辑返回
```

命中缓存时不再执行：

- 请求图片物化
- 请求体编码
- 上游 create 请求
- aiapidev poll 轮询
- 响应图片下载或上传

## 覆盖路径

首版覆盖以下路径：

1. Gemini 标准链路：`/v1beta/models/{model}:generateContent`
2. OpenAI image 链路：`/v1/images/generations`
3. aiapidev Gemini 任务链路：
   - create 非成功响应
   - poll 非重试型错误响应
   - poll 连续失败后返回的最后一次错误
   - task 终态失败时代理生成的 `502`
4. aiapidev OpenAI image 任务链路：
   - create 非成功响应
   - poll 非重试型错误响应
   - poll 连续失败后返回的最后一次错误
   - task 终态失败时代理生成的 `502`

## 错误处理设计

缓存命中时必须返回与首次可缓存错误一致的核心响应：

```text
status code: 与首次响应一致
content-type: 与首次响应一致，缺省为 application/json
body: 与首次响应一致
```

不额外包装错误，不改写 message，不新增下游可见字段。

如果缓存读取、序列化或 key 生成出现代理内部错误，不能影响正常请求转发；应当 fail-open，继续请求上游。

## Admin 与观测

首版不要求新增 admin UI。

但 admin 日志应保持可排障：

1. 缓存命中请求仍记录一条日志。
2. status code 与响应体按实际返回记录。
3. 首版在 `errorDetail` 中标记 `upstream_block_cache_hit:<reason>`，用于区分“真实上游返回”和“代理缓存命中”。
4. 缓存命中不增加现有请求图片 cache hit 统计，避免混淆两类缓存。

首版不新增前端展示，只保证日志详情里能看出命中来源。

## 配置设计

新增配置字段：

```text
upstream_block_cache_ttl: Duration
upstream_block_cache_max_entries: usize
```

新增环境变量：

```text
UPSTREAM_BLOCK_CACHE_TTL_MS
UPSTREAM_BLOCK_CACHE_MAX_ENTRIES
```

默认值：

```text
UPSTREAM_BLOCK_CACHE_TTL_MS=300000
UPSTREAM_BLOCK_CACHE_MAX_ENTRIES=1024
```

边界规则：

- TTL 为 `0` 时不创建缓存实例。
- max entries 为 `0` 时也不创建缓存实例。
- 非法数值使用默认值，保持现有配置解析风格。

## 并发与资源控制

缓存结构应放在 `AppState` 中，以 `Arc` 共享。

并发要求：

1. 多个请求同时读写缓存时不能 panic。
2. 缓存命中路径不能持有锁执行昂贵操作。
3. 写入条目时 body 应限制为上游错误响应体实际大小；当前错误响应通常很小，不需要额外引入大对象存储。
4. 如果未来发现上游错误体可能很大，再补充最大 body 缓存尺寸。

## 测试策略

新增或扩展测试时，后台执行单元测试必须设置最大 60 秒超时。

重点测试：

1. 第一次违规请求转发上游，并把错误写入缓存。
2. 第二次相同请求直接返回缓存错误，上游请求计数不增加。
3. 不同请求体不会命中。
4. 不同上游 base_url 不会命中。
5. 相同请求使用不同 API key 仍能命中。
6. TTL 为 `0` 时不缓存。
7. 非违规 `400` 不缓存。
8. 非违规 `502` 不缓存。
9. `content blocked`、`image_unsafe`、`Upstream moderation triggered` 都能触发缓存。
10. aiapidev task 终态失败中的违规错误能触发缓存。

建议验证命令：

```bash
timeout 60s cargo test
```

## 风险控制

1. 只缓存上游已经明确返回过的违规错误，避免首请求误拦截。
2. 默认 TTL 较短，减少上游策略变化后的长期影响。
3. key 包含完整规范化请求体，避免只按 prompt 文本造成误伤。
4. key 不包含 API key，这会跨 key 共享违规判定；这是本需求明确选择的资源节省策略。
5. 不缓存普通 `429/500/503/504`，避免把临时故障放大为固定失败。

## 验收标准

完成实现后应满足：

1. 默认开启 5 分钟上游违规错误缓存。
2. 相同违规请求在 TTL 内不会二次请求上游。
3. 缓存命中返回的 status、content-type 和 body 与首次错误一致。
4. 不同请求体或不同上游 base_url 不串缓存。
5. TTL 或最大条目数为 `0` 时机制关闭。
6. 非违规错误不缓存。
7. `timeout 60s cargo test` 通过。

## 后续路线

如果该机制上线后确认收益明显，可以后续单独评估：

1. 在 admin stats 中增加 block-cache hit/miss 指标。
2. 增加可配置关键词列表。
3. 增加可配置最大缓存 body 大小。
4. 对多实例部署引入共享缓存，但这需要单独评估一致性、隔离性和运维成本。
