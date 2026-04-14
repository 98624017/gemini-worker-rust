# Header Dual Key Design

**目标**

在 `rust-sync-proxy` 中复刻 Go 版生产行为里的“请求头双 key 路由”能力，但范围只限于**请求头覆盖**，不扩展到环境变量或配置项。

**范围**

- 支持现有单 key 形式：
  - `<apiKey>`
  - `<baseUrl>|<apiKey>`
- 新增双 key 形式：
  - `<baseUrl1>|<apiKey1>,<baseUrl2>|<apiKey2>`
- Header 优先级保持不变：
  - `x-goog-api-key` > `Authorization: Bearer ...`
- 路由规则：
  - 当请求体 `generationConfig.imageConfig.imageSize` 为 `4k` 或 `4K` 时，选择第二组上游
  - 其他情况选择第一组上游

**设计原则**

- 只在请求头覆盖链路中支持双 key，不新增环境变量语义
- 双 key 只负责先选出最终 `base_url + api_key`
- 选中最终上游后，后续仍按现有逻辑判断是标准上游还是 `aiapidev` 特殊上游
- 非法双 key 不做静默降级，返回客户端错误

**实现方式**

采用最小侵入方案：

1. 在 `upstream.rs` 新增按请求体上下文解析请求头 token 的能力
2. 在 `router.rs` 中把“读请求体 JSON”提前到上游解析之前
3. 解析出最终上游后，复用现有标准链路 / 特殊上游链路，不重写后续流程

**错误处理**

- 缺少上游 key：保持 `401`
- token 含逗号但不能解析成两组合法 `<baseUrl>|<apiKey>`：返回 `400`
- 自定义 `baseUrl` 非法：返回 `400`

**测试策略**

- 先补 `upstream_auth_test.rs`：
  - `4k` 选择第二组
  - 非 `4k` 选择第一组
  - 非法双 key 返回错误
- 再补 `http_forwarding_test.rs`：
  - 第二组是标准上游时，请求命中第二个 mock upstream
  - 第二组是 `aiapidev` 时，仍然走特殊上游链路

**影响面**

- 直接修改：
  - `src/upstream.rs`
  - `src/http/router.rs`
  - `tests/upstream_auth_test.rs`
  - `tests/http_forwarding_test.rs`
  - `README.md`
- 不修改：
  - 配置结构
  - admin 鉴权逻辑
  - 请求侧缓存 / 图片上传逻辑
