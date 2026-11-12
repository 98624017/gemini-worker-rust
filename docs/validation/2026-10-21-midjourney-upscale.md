# Midjourney 指定候选图与高清放大验证

测试日期：2026-10-21（Asia/Shanghai）。直连 `https://magic666.top`，使用本次已成功的 v8.1 Imagine 任务 `1792589432444193`。密钥通过隐藏输入进入测试进程，未写入脚本或报告。

## 结论

`mj_fast_upscale` 对应的 `UPSCALE` 操作可以通过 `taskId` 和 `index` 指定 U1–U4。本次 U1、U3 均成功，但返回的是原有 **1024 × 1024** 单图。U1 的下载文件与原单图字节完全相同，不能将这一步计为提高分辨率。

随后从选图任务的 `buttons` 读取 `Upscale (2x)` / `Upscale (4x)`，原样提交对应 `customId`。这两次任务实际返回的动作是 `UPSAMPLE_ACTION`，按本网关动作命名对应 `mj_fast_upsample_action`；均进入 `FAILURE`，原因是 `无效生图请求`，没有获得高清图。

因此，本次验证通过的是“指定候选图”，2 倍 / 4 倍高清放大尚未跑通。请求里没有显式 `model` 字段，模型名称依据网关已有模型列表和实际返回的动作命名；未查询计费日志。

## 来源与结果

magic 的 [提交文档](https://docs.magic666.top/api-reference/images/midjourney/submit) 未完整展开 change / action 路由，但列出了 `task_id_is_required`、`action_is_required`、`index_is_required` 等操作校验错误。[查询文档](https://docs.magic666.top/api-reference/images/midjourney/query) 说明了 `UPSCALE` 和 `buttons[].customId`。

补充参考兼容实现的 [路由注册](https://github.com/QuantumNous/new-api/blob/main/router/relay-router.go)、[Midjourney 请求结构](https://github.com/QuantumNous/new-api/blob/main/dto/midjourney.go) 和 [按钮动作转换](https://github.com/QuantumNous/new-api/blob/main/service/midjourney.go)，确认 `/mj/submit/change`、`/mj/submit/action` 及请求字段。该兼容实现不代表 magic 部署版本，以下结论以 magic 的真实响应为准。

| 操作 | 新任务 ID | 实际动作 | 结果 |
| --- | --- | --- | --- |
| 原任务 U1，`index=1` | `1792593981515112` | `UPSCALE` | `SUCCESS`；14.81 秒完成并下载；1024 × 1024 |
| 对 U1 执行 2x | `1792594150408709` | `UPSAMPLE_ACTION` | `FAILURE`；`无效生图请求` |
| 对 U1 执行 4x | `1792594221784270` | `UPSAMPLE_ACTION` | `FAILURE`；`无效生图请求` |
| 原任务 U3，`index=3` | `1792594278260648` | `UPSCALE` | `SUCCESS`；20.64 秒完成并下载；1024 × 1024 |

本次共提交 4 次请求：两次选图成功、两次高清操作失败。所有请求都取得任务 ID，失败发生在后续任务执行阶段。

## 指定放大哪一张：taskId + index

先确认原任务 `status=SUCCESS`，然后发送：

```http
POST https://magic666.top/mj/submit/change
Authorization: Bearer YOUR_API_KEY
Content-Type: application/json
```

```json
{
  "mode": "FAST",
  "taskId": "1792589432444193",
  "action": "UPSCALE",
  "index": 3
}
```

`index` 的四宫格位置：

| index / 按钮 | 位置 |
| --- | --- |
| 1 / U1 | 左上 |
| 2 / U2 | 右上 |
| 3 / U3 | 左下 |
| 4 / U4 | 右下 |

本次 `index=3` 的提交响应：

```json
{"code":1,"description":"选图操作成功","result":"1792594278260648","properties":{}}
```

使用**新任务 ID** 查询 `GET /mj/task/1792594278260648/fetch`，直到 `status=SUCCESS`，从 `imageUrl` 下载结果。

- [U1 选图结果](https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png)：1,537,353 字节，1024 × 1024。SHA-256 为 `1cf18f817124ffc987e8f45c7cfe63eea75bc530cf42c06414685abd917c94eb`，与前序下载的原始单图完全相同，`cmp` 校验通过。
- [U3 选图结果](https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_2.png)：1,340,818 字节，1024 × 1024；URL 与原任务 `imageUrls[2].url` 相同，确认选择了非默认候选图。

## 指定 2 倍 / 4 倍：使用返回的按钮

选图成功后，再查询该选图任务。本次 U1 任务 `1792593981515112` 返回：

```json
[
  {
    "label": "Upscale (2x)",
    "customId": "MJ::JOB::upsample_v5_2x::1::6ad8be78f3256cf1217495aa::SOLO"
  },
  {
    "label": "Upscale (4x)",
    "customId": "MJ::JOB::upsample_v5_4x::1::6ad8be78f3256cf1217495aa::SOLO"
  }
]
```

向 `POST https://magic666.top/mj/submit/action` 发送以下请求，这是本次已提交但最终执行失败的 2x 请求：

```json
{
  "mode": "FAST",
  "taskId": "1792593981515112",
  "customId": "MJ::JOB::upsample_v5_2x::1::6ad8be78f3256cf1217495aa::SOLO"
}
```

4x 使用同一选图任务返回的 `Upscale (4x)` 的 `customId`。其他任务必须先查询自己的按钮；不要复用本例的 ID，也不要自行把按钮名里的 `v5` 改为 `v8.1`，这些是服务端返回的完整操作标识。

两次提交均返回 `code=1`，但轮询约 13–15 秒后得到：

```json
{
  "action": "UPSAMPLE_ACTION",
  "status": "FAILURE",
  "progress": "100%",
  "failReason": "无效生图请求"
}
```

因此 `code=1` 和 `progress=100%` 均不能单独作为高清放大成功的依据。失败任务的最终提示词仍包含原来的 `--v 8.1`，但没有输出图片，不能确认该线路支持 v8.1 的高清操作，也无法从这段错误确定其内部失败原因。

## 复用探针

`scripts/midjourney_smoke.py` 新增选图和高清按钮操作，提交前重新查询源任务状态；高清操作只使用响应中实际存在的按钮。

```bash
rtk proxy python3 scripts/midjourney_smoke.py \
  --case upscale --parent-task-id 1792589432444193 --index 3 \
  --output-dir /tmp/mj-upscale-new/u3

rtk proxy python3 scripts/midjourney_smoke.py \
  --case upscale-hd --parent-task-id 1792593981515112 --scale 2 \
  --output-dir /tmp/mj-upscale-new/2x
```

第一条选择已有四宫格的 U3；第二条尝试已有 U1 单图的 2x 操作，本次后者未通过。`--parent-task-id` 指操作对象，`--task-id` 则只恢复查询已提交的新任务，避免重复提交。

原始请求、源任务、提交响应和最终任务保存在 `/tmp/mj-v81-smoke-20261021/upscale-u1/`、`upscale-u3/`、`upscale-2x/`、`upscale-4x/`。通过脚本自检、Ruff 和格式检查；成功单图均实际下载和解码。未修改 Rust 业务代码，未运行 Rust 全量测试，未验证 U2 / U4 的实际提交或其他版本的高清放大。
