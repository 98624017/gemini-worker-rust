# 代理错误建模与报错信息增强设计

日期: 2026-04-11
状态: 已批准

## 目标

为 `rust-sync-proxy` 建立统一的错误建模规则，改善当前标准渠道和
`aiapidev` 特殊渠道的报错一致性、对下游可读性和对 admin 排障可观测性。

本次设计目标：

1. 上游有明确错误时，以下游透传上游语义为主
2. 本层错误统一建模，避免直接把底层 `err.to_string()` 暴露给下游
3. 下游错误响应稳定包含 `code`、`message`、`source`、`stage`、`kind`
4. admin 日志补齐错误阶段、类型、原始细节和上游错误摘录
5. 第一轮覆盖标准渠道主链路和 `aiapidev` 特殊分支

## 非目标

本次不做以下事项：

- 不重做 admin UI 整体布局
- 不引入完整 trace id / request id 跨系统追踪体系
- 不对项目里所有辅助模块的每一个错误点都做细粒度分类
- 不做国际化或中英双语错误文案体系
- 不改变“上游业务错误优先透传”的总体原则

## 现状问题

当前项目里的错误处理存在三个主要问题：

1. 标准渠道和 `aiapidev` 分支的错误风格不统一
2. 很多本层 `502` 直接返回底层库原始错误字符串，文案不稳定
3. admin 日志缺少“错误来源、阶段、类型、原始细节”这些关键诊断字段

例如标准渠道里的：

```json
{
  "error": {
    "code": 502,
    "message": "error decoding response body"
  }
}
```

这条信息只说明“坏了”，但不能稳定表达：

- 是上游错还是代理错
- 坏在连接、读响应头、读响应体还是解码阶段
- 是否是超时、断流、压缩解码失败或 JSON 解析失败

## 方案选择

本次评估了三条路径：

1. 只调整文案，不改错误模型
2. 建立统一的本层错误模型，上游错误继续优先透传
3. 在方案 2 基础上同步做完整 trace 和 admin 观测升级

最终选择方案 2。

原因：

- 能保持“上游错误优先透传”的原则
- 能把本层 502、超时、断流、解码失败统一成稳定结构
- 覆盖面足够完整，但不会膨胀成观测系统重构

## 错误边界

### 上游错误

定义：

- 上游已经返回明确 HTTP 响应
- 且这个响应表达的是上游自己的失败

处理原则：

- 优先保留上游状态码
- 优先保留上游 message / body 语义
- 仅补充最小统一字段，不覆盖上游原意

### 代理错误

定义：

- 请求尚未成功获得上游有效响应
- 或上游响应尚未被本层完整读完、解码完、解析完、改写完
- 或本层在材料化、上传、压缩、轮询、序列化等流程中失败

处理原则：

- 统一进入本层错误模型
- 对下游返回稳定文案和结构
- 原始错误细节只进入 admin 日志

## 对下游的错误结构

本次批准的统一结构为：

```json
{
  "error": {
    "code": 502,
    "message": "failed to decode upstream response body",
    "source": "proxy",
    "stage": "decode_upstream_body",
    "kind": "body_decode_failed"
  }
}
```

字段含义：

- `code`
  保持现有兼容字段
- `message`
  面向调用方的稳定错误文案
- `source`
  错误来源，取值 `proxy` 或 `upstream`
- `stage`
  错误发生阶段
- `kind`
  错误性质

原则：

- 下游暴露 `message/source/stage/kind`
- 不向下游直接暴露底层 `reqwest` / `hyper` / `serde_json` 原始错误全文

## `stage` 枚举

第一版批准以下阶段值：

- `read_request_body`
- `parse_request_json`
- `materialize_request`
- `encode_request_body`
- `connect_upstream`
- `send_upstream_request`
- `read_upstream_headers`
- `read_upstream_body`
- `decode_upstream_body`
- `parse_upstream_json`
- `rewrite_upstream_response`
- `finalize_response`
- `aiapidev_create_task`
- `aiapidev_poll_task`
- `aiapidev_parse_task_response`
- `upstream_response`

说明：

- `read_upstream_body` 表示已获得响应头，但在完整读取 body 时失败
- `decode_upstream_body` 表示压缩解码或底层 body 解码失败
- `parse_upstream_json` 仅用于“本层明确要求必须是 JSON”的场景

## `kind` 枚举

第一版批准以下类型值：

- `invalid_request`
- `request_too_large`
- `connection_failed`
- `tls_failed`
- `timeout`
- `request_body_write_failed`
- `response_header_timeout`
- `response_body_timeout`
- `body_truncated`
- `body_decode_failed`
- `invalid_json`
- `upstream_error`
- `response_rewrite_failed`
- `upload_failed`
- `internal_error`

## 典型映射示例

### 标准渠道 body 解码失败

当前现象：

```json
{
  "error": {
    "code": 502,
    "message": "error decoding response body"
  }
}
```

新结构：

```json
{
  "error": {
    "code": 502,
    "message": "failed to decode upstream response body",
    "source": "proxy",
    "stage": "decode_upstream_body",
    "kind": "body_decode_failed"
  }
}
```

### `aiapidev` 轮询超时

```json
{
  "error": {
    "code": 502,
    "message": "aiapidev task poll timed out",
    "source": "proxy",
    "stage": "aiapidev_poll_task",
    "kind": "timeout"
  }
}
```

### 上游直接返回 429

```json
{
  "error": {
    "code": 429,
    "message": "rate limited",
    "source": "upstream",
    "stage": "upstream_response",
    "kind": "upstream_error"
  }
}
```

## admin 日志增强

`AdminLogEntry` 第一版新增以下字段：

- `error_source`
- `error_stage`
- `error_kind`
- `error_message`
- `error_detail`
- `upstream_status_code`
- `upstream_error_body`

设计原则：

- `error_message` 与下游返回的稳定 message 保持一致
- `error_detail` 保存原始错误详情，仅用于内部排障
- `upstream_error_body` 做长度裁剪，避免写入完整大页或敏感内容

## admin UI 范围

第一版只做最低限度增强：

1. 列表增加基础错误标签
2. 详情页增加 `error` 区块
3. 如存在 `upstream_error_body`，单独展示裁剪内容

本轮不做复杂筛选、聚合分析和错误指纹去重。

## 第一轮实施范围

第一轮必须覆盖：

1. 标准渠道主链路
2. `aiapidev` 特殊分支
3. 上游显式错误透传出口
4. admin 记录和基础展示

第一轮暂不覆盖：

- 完整 trace id
- admin 大改版
- 全项目所有边缘模块的完整分类

## 验证标准

### 响应结构验证

- 本层错误响应包含 `code/message/source/stage/kind`
- 上游错误响应保留上游状态码和主要语义

### 诊断验证

- admin 日志能区分 `proxy` 和 `upstream`
- admin 日志能显示阶段、类型和原始细节

### 测试验证

至少补齐以下测试：

- 标准渠道连接 / 读 body / 解码 / 必需 JSON 解析失败
- 上游非 2xx 透传
- `aiapidev` create / poll / parse 失败
- admin 日志字段记录

