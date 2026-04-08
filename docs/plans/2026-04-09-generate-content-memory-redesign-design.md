# generateContent 内存重构设计

日期: 2026-04-09
状态: 已批准

## 目标

围绕真实主场景重构 Rust 同步代理的数据面：

- 只服务 `POST /v1beta/models/{model}:generateContent`
- 请求体里常见 `inlineData.data=http(s)://...` 的生图请求
- 上游返回非流式 JSON
- 下游主要使用 `output=url`

本次设计的核心目标不是“语言切换”，而是：

1. 在固定实例内存下承接更多并发
2. 不依赖 `429/503` 拒绝请求兜底
3. 不依赖长时间排队兜底
4. 正常请求不明显降低速度和吞吐

## 范围收缩

本次重构主动缩小边界，只保留真实高频主链路：

- 保留：`POST /v1beta/models/{model}:generateContent`
- 保留：请求侧图片 URL 拉取与回填
- 保留：响应侧 `output=url` 上传与回填
- 保留：R2 / legacy 上传
- 保留：基础配置解析与最小观测

本次重构明确移出范围：

- 删除 `streamGenerateContent`
- 删除 `/proxy/image`
- 删除“本仓库代理图床 URL 下载”的职责
- 删除“返回图床 URL 后再包装为本仓库 `/proxy/image`”的逻辑

最终返回图片 URL 的方式统一为：

- `EXTERNAL_IMAGE_PROXY_PREFIX + escaped(real_url)`

也就是说，本服务只负责：

- 拉取请求侧源图片
- 调上游
- 上传响应图片
- 输出外部代理前缀 URL

不再负责图片下载代理。

## 非目标

以下内容不在本次设计范围内：

- 与 Go 版维持完整 API 对等
- 继续保留 SSE / 流式返回兼容
- 实现更复杂的 `/proxy/image` 安全能力
- 解决所有非图片路径的性能问题
- 引入异步任务系统或显式队列

## 现状问题

当前实现最伤内存的地方，集中在你们真实主路径上：

1. 请求体整包读入，再反序列化，再重编码发给上游
2. 请求侧图片 URL 拉取后，需要生成更大的 base64 文本
3. 非流式响应整包读入，再解 JSON，再改写，再重编码
4. `output=url` 时，又把 base64 解码回图片字节再上传
5. 大对象经常以多种形式同时存在：
   - 原图 bytes
   - base64 string
   - 完整 JSON body
   - 上传 body 副本

当前瓶颈不是 HTTP 转发本身，而是：

- 大对象生命周期过长
- 多份中间态共存
- 所有阶段默认把大对象放在 RAM 里

## 设计总览

### 核心思想

引入统一的大对象抽象 `BlobHandle`，把“业务结构”和“大数据载荷”分离。

后续各阶段不再直接传递：

- `Vec<u8>`
- 大 `String`
- 含大 base64 的完整 `serde_json::Value`

而是统一传递：

- 轻量 JSON 结构
- `BlobHandle`

### `BlobHandle` 语义

`BlobHandle` 是大对象的统一句柄，内部可有两种形态：

- `Inline(bytes)`：小对象直接保存在内存
- `Spilled(path)`：大对象落到临时文件

同时附带元数据：

- MIME
- 大小
- 可选 sha256
- 来源 URL
- 生命周期状态

业务层不关心对象落在内存还是文件；是否 spill 完全由运行时预算决定。

## 模块设计

本次重构建议按“数据阶段”拆模块，而不是按旧功能文件简单修补。

### 1. `ingress`

负责：

- 路由
- 鉴权
- 请求大小上限
- 请求上下文与 trace id

仅保留 `generateContent`。

### 2. `request_scan`

负责：

- 轻量定位请求 JSON 中的 `inlineData.data=http(s)://...`
- 记录待替换节点位置

不在这里下载图片，也不生成大 base64。

### 3. `blob_runtime`

这是本次重构的中心组件，负责：

- 依据预算创建 blob sink
- 决定对象走内存还是 spill
- 管理临时文件目录
- 打开 reader / writer
- 请求结束回收
- 进程重启后的遗留清理

### 4. `image_fetch`

负责请求侧图片 URL 拉取：

- 输入：图片 URL
- 输出：`BlobHandle`

内部按块读取，直接写入 `blob_runtime` 分配的 sink。

### 5. `request_encode`

负责把“轻量 JSON 骨架 + BlobHandle”编码成发送给上游的请求体。

关键要求：

- 不先构造完整大 base64 `String`
- 不先构造完整含图片的巨大 `serde_json::Value`
- 直接流式写出最终上游请求 body

### 6. `upstream_forward`

负责把 `request_encode` 产出的 body 发给上游，并获取非流式响应。

这一层只关心传输，不处理图片语义。

### 7. `response_scan`

负责从上游非流式 JSON 里定位：

- 输出图片
- 文本
- finishReason
- 其他小结构字段

目标是尽量缩短大 base64 在内存中的停留时间。

### 8. `response_materialize`

把响应里的 base64 图片直接解码进新的 `BlobHandle`。

关键原则：

- 一旦图片 materialize 成 `BlobHandle`
- 原始大 base64 应尽快释放

### 9. `upload_finalize`

从 `BlobHandle` 直接上传到 legacy / R2。

上传成功后：

- 拿到真实 URL
- 再改写成 `EXTERNAL_IMAGE_PROXY_PREFIX + escaped(real_url)`
- 回填到最终响应 JSON

### 10. `response_encode`

负责输出最终下游响应，只输出最终 URL，不再保留大 base64。

## 数据流

### 请求侧

1. 入口接收 `generateContent` 请求
2. `request_scan` 识别图片 URL 占位
3. `image_fetch` 拉取图片并写入 `BlobHandle`
4. `request_encode` 读取 `BlobHandle`，边 base64 编码边写出上游请求 JSON
5. `upstream_forward` 把流式请求体发给上游

### 响应侧

1. 获取上游非流式 JSON 响应
2. `response_scan` 定位图片 base64 与其他字段
3. `response_materialize` 把图片解码为 `BlobHandle`
4. `upload_finalize` 从 `BlobHandle` 直接上传
5. 取得真实 URL 后，拼接外部代理前缀
6. `response_encode` 输出最终 JSON

## 峰值内存模型

本次设计必须从机制上避免以下两类长期共存：

### 请求侧禁止长期共存

- 原图 bytes
- base64 string
- 完整上游请求 body

### 响应侧禁止长期共存

- 上游 base64 string
- 解码后图片 bytes
- 上传 body 副本

重构后的目标不是“绝对零拷贝”，而是把峰值从“多份大对象驻留内存”改成：

- 小图：以内存直通为主
- 大图：固定小缓冲 + spill 文件

## 热内存预算模型

这次不采用“请求数限流”，而采用“按字节预算切换介质”。

### 三层预算

1. `BLOB_INLINE_MAX_BYTES`
   单个 blob 走内存快路径的上限

2. `REQUEST_HOT_MEMORY_BUDGET_BYTES`
   单请求允许占用的热内存预算

3. `GLOBAL_HOT_MEMORY_BUDGET_BYTES`
   实例级热内存总预算

### 行为原则

- 未超预算：新 blob 允许进内存
- 超单请求预算：该请求后续新 blob 自动 spill
- 超全局预算：全局后续新大对象优先 spill
- 不拒绝请求，不显式排队，只切换存储介质

这使 spill 成为保护机制，而不是常态路径。

## 默认参数建议

### 面向主流实例规格的建议值

#### 2GiB

- `BLOB_INLINE_MAX_BYTES = 8MiB`
- `REQUEST_HOT_MEMORY_BUDGET_BYTES = 24MiB`
- `GLOBAL_HOT_MEMORY_BUDGET_BYTES = 384MiB`

#### 4GiB

- `BLOB_INLINE_MAX_BYTES = 12MiB`
- `REQUEST_HOT_MEMORY_BUDGET_BYTES = 40MiB`
- `GLOBAL_HOT_MEMORY_BUDGET_BYTES = 768MiB`

#### 8GiB

- `BLOB_INLINE_MAX_BYTES = 16MiB`
- `REQUEST_HOT_MEMORY_BUDGET_BYTES = 64MiB`
- `GLOBAL_HOT_MEMORY_BUDGET_BYTES = 1536MiB`

### 自适应公式

如果采用自动计算，建议：

- `BLOB_INLINE_MAX_BYTES = min(16MiB, max(8MiB, 容器内存 / 512))`
- `REQUEST_HOT_MEMORY_BUDGET_BYTES = min(64MiB, max(24MiB, 容器内存 / 128))`
- `GLOBAL_HOT_MEMORY_BUDGET_BYTES = min(1536MiB, max(384MiB, 容器内存 / 5))`

设计意图：

- 你们常见的 `2MiB+` 图片仍然能走快路径
- spill 只在“大图叠加 + 并发上来”时触发
- 正常请求速度不被普遍拖慢

## 错误处理

### 请求侧图片获取失败

保持 `fail-closed`：

- 拉取失败
- 超时
- 超限

直接返回错误，不发给上游。

### 上游失败

直接透传状态码和错误体。

### `output=url` 上传失败

建议保留双模式：

- 默认：`fail-open`，保留原始 base64
- 严格模式：直接返回错误

默认仍用 `fail-open`，避免线上兼容性回归。

### 外部代理前缀配置错误

启动时失败，不作为运行时动态错误处理。

## 资源清理

### 请求级清理

每个请求维护一个 `RequestScope`：

- 记录本请求创建的全部 `BlobHandle`
- 请求成功结束时统一回收
- 请求中途失败时同样兜底回收

### 进程级清理

启动时扫描 spill 目录：

- 按命名前缀识别本服务临时文件
- 按 TTL 清理遗留文件

必须保证 spill 文件不会无限积累。

## 性能策略

为了满足“不明显降低速度吞吐”，必须坚持双路径：

### 快路径

- 常见图片走内存
- 普通请求不触发 spill
- 延迟应尽量接近当前实现

### 保护路径

- 预算紧张时自动 spill
- 大请求继续处理，但把 RAM 压力转移到文件系统

### 关键约束

- 不能把所有请求都强制 spill
- 不能再允许所有大对象默认长期占用内存

## 观测指标

至少新增以下指标：

- 当前 hot memory 使用量
- spill 次数
- spill 总字节数
- spill 文件数
- 请求侧图片下载耗时
- 上游请求编码耗时
- 响应图片 materialize 耗时
- 上传耗时
- 上传失败率
- 峰值 RSS

默认日志只记录元数据，不默认记录完整大 payload。

## 验证标准

### 功能

- `generateContent` 主链路行为正确
- 请求侧图片 URL 正常工作
- 非流式 `output=url` 正常工作
- 外部代理前缀拼接正确

### 性能

- 小图请求 P95 不显著劣化
- 常见图片大小下吞吐不明显下降

### 承载

- 同规格实例的安全并发显著高于当前实现
- 大图混合流量下 RSS 更平稳，不易逼近 OOM

### 资源

- spill 无泄漏
- 请求取消后及时回收
- 进程重启后遗留文件可清理

## 实施顺序

1. 收缩范围
   - 删除 `streamGenerateContent`
   - 删除 `/proxy/image`
   - 删除本地代理 URL 包装逻辑

2. 引入 `blob_runtime`
   - 先实现统一句柄、预算、spill、清理

3. 重写请求侧
   - `request_scan`
   - `image_fetch`
   - `request_encode`

4. 重写响应侧
   - `response_scan`
   - `response_materialize`
   - `upload_finalize`

5. 最后补观测与调参

## 最终结论

这次重构的本质不是“把 Go 翻译成 Rust”，而是：

- 删除你们不需要的路径
- 只保留 `generateContent` 主链路
- 用 `BlobHandle + spill runtime` 重建图片数据面

只有这样，才能在不依赖拒绝请求的前提下，把固定内存实例的可承载并发真正做上去。
