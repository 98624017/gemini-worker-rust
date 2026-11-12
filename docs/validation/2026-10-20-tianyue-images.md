# 天悦同步与异步图像接口验证

测试日期：2026-10-20（Asia/Shanghai）。测试目标为 `https://api.tianyue.xyz/v1/images/generations`，使用本次修改后本地构建的代理。API key 仅通过隐藏输入载入测试进程内存，不写入仓库、请求样例或报告。

## 修改与本地验证

- `api.tianyue.xyz` 的 `seedream-5-pro` 按 `size=1K/2K` 改写为 `seedream-5-pro-1K/2K`，去掉原来的 `GZ-` 前缀。
- 保留参考图别名合并逻辑；`duration`、`aspect_ratio`、`output_resolution` 原样透传。
- `rtk cargo test --test openai_image_test`：9 项通过。
- `rtk cargo test --lib tianyue`：2 项通过，包括模拟上游的完整请求改写和结果代理链路。
- `rtk cargo fmt --check`、`rtk git diff --check`、`rtk cargo build` 通过。

## 参考图与提示词

参考图：<https://upload.wikimedia.org/wikipedia/commons/3/3a/Cat03.jpg>。本地下载检查为 HTTP 200、`image/jpeg`、1600 × 1598 像素，主体为橘色虎斑猫。

统一提示词：

> 请以参考图中的橘色虎斑猫为同一只主角，保留它的脸型、额头虎斑纹、黄绿色眼睛、粉色鼻子和白色口鼻。让它坐在明亮的摄影棚里，头颈清晰可见，居中构图，完整保留双耳，背景为柔和浅蓝色，无其他动物，无文字、无水印。写实宠物摄影，柔和自然光，毛发细节清晰。根据输出画布的比例自然调整留白，不要拉伸猫的身体。

## Seedream 实测

`GET /v1/models` 返回 HTTP 200，模型列表包含 `seedream-5-pro-1K`、`seedream-5-pro-2K` 和 `gpt-image-2-yb`。模型被列出不代表当时生成线路可用。

六组请求均通过本地代理发送，输入 `model=seedream-5-pro`，同时传入对应的 `size`、`output_resolution`、`aspect_ratio` 和 `duration=1`。三种参考图字段分别为 `image`、`reference_images`、`images`，由代理统一为 `images`。

| 改写后的模型 | 比例 | HTTP 状态 | 响应中的上游状态 | 返回图片 |
| --- | --- | --- | --- | --- |
| `seedream-5-pro-1K` | `1:1` | 500 | 503 | 无 |
| `seedream-5-pro-1K` | `16:9` | 500 | 503 | 无 |
| `seedream-5-pro-1K` | `9:16` | 500 | 503 | 无 |
| `seedream-5-pro-2K` | `1:1` | 500 | 503 | 无 |
| `seedream-5-pro-2K` | `16:9` | 500 | 503 | 无 |
| `seedream-5-pro-2K` | `9:16` | 500 | 503 | 无 |

六组均返回：

```json
{
  "error": {
    "message": "上游创建任务失败，状态码:503",
    "type": "upstream_error",
    "param": "",
    "code": "task_failed"
  }
}
```

另将 `seedream-5-pro-1K` 直接请求天悦，移除代理额外的 `size` 和 `response_format`，只保留文档参数及相同参考图，仍返回 HTTP 500 / 上游 503。再直连 `seedream-5-pro-2K` 并移除参考图，也返回相同错误。因此，该次失败也能在绕过本地代理、移除参考图的情况下复现；尚不能仅凭这一响应确定天悦内部失败原因。

未获得 Seedream 生成图，无法确认实际输出像素、比例和参考图一致性。上述六组是已完成的失败实测，不能视为生成能力验证通过。

## 文档示例模型对照

使用文档示例模型 `gpt-image-2-yb` 直连相同同步接口：

- Wikimedia 参考图搭配 `1K / 1:1` 或 `2K / 9:16`，均返回 HTTP 500，错误内容为“上游创建任务失败，状态码:400”。
- `2K / 9:16` 移除参考图后成功，生成请求耗时 36.68 秒，返回图片下载 HTTP 200、`image/png`，实际尺寸为 **720 × 1280**。比例为 `9:16`，但请求的 `2K` 不能据此判定有效。
- 无参考图成功样例的目视检查：浅蓝背景、橘色虎斑猫、完整双耳、无可见文字；因为未传参考图，不能用于判断参考图一致性。

补充参考图选用 Unsplash 黑白双色猫：<https://images.unsplash.com/photo-1514888286974-6c03e2ca1dba?w=1024&q=85&fit=crop>。本地下载检查为 HTTP 200、`image/jpeg`、1024 × 705 像素。测试该图时，提示词相应改为保留黑白双色毛色、绿色眼睛、粉色鼻子、白色口鼻与胸前毛色。

该 Unsplash 参考图的 `2K / 9:16` 直连请求成功，生成耗时 114.82 秒，返回图片下载 HTTP 200、`image/png`，实际尺寸仍为 **720 × 1280**。目视对照保留了黑色头顶、双耳、绿色眼睛、粉色鼻子、白色口鼻与胸前毛色，背景改成浅蓝，双耳完整。脸型、耳朵比例和毛色边界仍有变化，不据此宣称精确还原同一只猫。

经过本地代理的 `1K / 1:1` 无参考图请求也成功，生成耗时 96.86 秒，下载为 HTTP 200、`image/png`、**1024 × 1024**。

换图后成功说明该示例模型的同步参考图流程可用；Wikimedia 请求失败的内部原因仍未确定。两次请求 `2K / 9:16` 均未获得 2K 像素图，不能把接口返回成功等同于分辨率参数已生效。

最终 `2K / 16:9` 无参考图请求经过本地代理也成功，生成耗时 90.34 秒，下载为 HTTP 200、`image/png`、**1280 × 720**。

| 模型 | 请求分辨率 | 请求比例 | 参考图 | 链路 | 实际像素 | 生成耗时 |
| --- | --- | --- | --- | --- | --- | --- |
| `gpt-image-2-yb` | `1K` | `1:1` | 无 | 本地代理 | 1024 × 1024 | 96.86 秒 |
| `gpt-image-2-yb` | `2K` | `9:16` | 无 | 直连 | 720 × 1280 | 36.68 秒 |
| `gpt-image-2-yb` | `2K` | `9:16` | Unsplash 猫图 | 直连 | 720 × 1280 | 114.82 秒 |
| `gpt-image-2-yb` | `2K` | `16:9` | 无 | 本地代理 | 1280 × 720 | 90.34 秒 |

本次共提交 14 次同步生成请求：Seedream 8 次均失败；文档示例模型 6 次中 4 次成功、2 次 Wikimedia 参考图请求失败。没有轮询任务状态，也没有在本地对输出图片缩放或裁切。所有成功结果均实际下载并解码检查像素尺寸。

结论：本地模型改写和参数透传验证通过；当时的 Seedream 生成线路未通过实测。文档示例模型可完成同步文生图和部分图源的参考图生成，三种比例均匹配，但本次 `2K` 请求只返回 720p 图像，不能作为 2K 输出已生效的依据。

## 异步接口补充实测

同日根据补充文档，直连 `POST /v1/video/generations`。请求只使用文档字段：`model`、`prompt`、`duration=1`、`aspect_ratio`、`output_resolution`、`images`，参考图使用同步测试已成功的 Unsplash 黑白猫图，不传 `size`、`response_format`。

| 提交模型 | 分辨率 / 比例 | 提交 HTTP 状态 | 是否取得任务 ID |
| --- | --- | --- | --- |
| `seedream-5-pro-1K` | `1K / 1:1` | 500 | 否 |
| `seedream-5-pro-2K` | `2K / 16:9` | 500 | 否 |
| `PIX-seedream-5-pro-2K-yb` | `2K / 16:9` | 200 | 是，随后失败 |

两次提交均返回 `code=fail_to_fetch_task`。解析 `message` 及其嵌套 `detail` 后，得到更具体的错误：

```json
{
  "code": "model_not_found",
  "message": "No available channel for model seedream-5.0-pro ..."
}
```

错误短句省略了账户组和请求追踪信息。可以确认本次请求被上游转为 `seedream-5.0-pro`，其路由没有可用渠道；失败发生在提交阶段，未取得可查询的任务 ID。仅凭该响应不能判定渠道是缺失、禁用、容量不足，还是其他配置原因。

当前 `/v1/models` 列出的 Seedream 型号中，只有 `PIX-seedream-5-pro-2K-yb` 符合异步文档的 `-yb` 要求。补充使用该型号、相同 Unsplash 参考图和 `2K / 16:9` 提交，返回 HTTP 200，成功取得任务 ID；后续按文档每 4 秒查询一次 `GET /v1/video/generations/{task_id}`。

该任务的 ID 为 `task_GnjK5KH7ZAKhmaW23QhUo73xph2asBb1`。提交后约 4.45 秒首次查询就返回 `data.status=FAILURE`、`data.progress=100%`，没有生成图片。解析 `data.fail_reason` 后为：

```json
{
  "error": {
    "message": "No active tokens available in the pool",
    "type": "server_error",
    "param": "",
    "code": "ERR-D2290E5F38"
  }
}
```

实际失败终态为 **`FAILURE`**，并非文档枚举中的 `FAILED`。最初按文档枚举运行的探针因此多查了数次；确认失败终态后停止轮询。未来接入轮询时必须兼容此实际状态，且不能把顶层 `code=success` 或 `progress=100%` 当作生成成功。

本次异步补充共提交 3 个任务：两个普通型号在提交阶段因上游 `seedream-5.0-pro` 没有可用渠道而失败；`PIX-...-yb` 型号提交成功，但因上游令牌池没有可用令牌而失败。均未得到成功生成态或可下载图片，无法验证其输出分辨率、比例和参考图效果。
