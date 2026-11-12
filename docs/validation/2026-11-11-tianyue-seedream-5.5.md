# 天悦 Seedream 5.5 同步参考图生成验证

测试日期：2026-11-11（Asia/Shanghai）。目标为 `https://api.tianyue.xyz/v1/images/generations`，请求经过本次修改后本地构建的代理。凭据通过隐藏终端输入载入测试进程内存，不写入源码、请求样例或报告。

结果：参考项目 Seedream 5 Pro 的九种比例均已实测，八种成功取得并下载图片；`9:21` 被天悦上游以 HTTP 400 / `invalid_aspect_ratio` 拒绝。当前 Seedream 5.5 在该渠道可复用现有同步参考图机制，但比例集合与 Seedream 5 Pro 存在这一项差异。

## 实现与本地验证

- 仅当上游 host 为 `api.tianyue.xyz` 时，将本层 `model=seedream-5.5` 固定改写为上游 `GZ-seedream-5.5-2K`，不依赖 `size`。
- 保留既有 `seedream-5-pro` 的模型改写；已有上游模型名不再改写，其他渠道不应用新映射。
- 复用参考图字段合并、参数透传、OpenAI 响应构造和天悦图片 URL 包装逻辑。
- `rtk cargo test --test openai_image_test`：10 项通过，覆盖新别名有无 `size`、其他渠道隔离、已有上游名以及参考图归一化。
- `rtk cargo test --lib tianyue`：2 项通过。其中请求/响应集成测试包含既有 Seedream 5 和新 Seedream 5.5 的四组用例，新模型覆盖三种参考图别名与三种比例。
- `rtk cargo fmt --check`、`rtk git diff --check`、`rtk cargo build` 通过。

## 真实请求与验证方法

`GET /v1/models` 返回 HTTP 200，包含 `GZ-seedream-5.5-2K`。生成测试统一使用同一张参考图，固定 `size=2K`、`output_resolution=2K`、`duration=1` 和 `response_format=url`。通过 `image`、`images`、`reference_images` 传入单张图片。

比例清单取自 `/root/projects/AIwork_Personal/src/shared/gpt-image-2-oai-size.ts:78` 中 Seedream 5 Pro 的模型专属配置，同文件的 UI 选项和主进程请求校验复用这份定义；`xinbao-request-contract.ts:272` 将选定比例发送为 `aspect_ratio`。完整九项为 `9:21`、`9:16`、`2:3`、`3:4`、`1:1`、`4:3`、`3:2`、`16:9`、`21:9`。该模型不含 `auto`、`5:4`、`4:5`。

先完成的 `1:1`、`16:9`、`9:16` 已成功取得并下载图片，扩展测试时复用这三组结果，只对其余六种比例提交新请求。

参考图：<https://images.unsplash.com/photo-1514888286974-6c03e2ca1dba?w=1024&q=85&fit=crop>。下载为 HTTP 200、JPEG、1024 × 705 像素，主体为绿背景前的黑白猫，绿色眼睛、粉色鼻子。

统一提示词：

> 请以参考图中的猫为主角，保留其毛色分布、脸部花纹、眼睛颜色和粉色鼻子。让它坐在明亮的摄影棚里，头颈清晰可见，完整保留双耳，背景为柔和浅蓝色，无其他动物，无文字，无水印。写实宠物摄影，柔和自然光，毛发细节清晰。根据请求的画布比例自然调整留白，不要拉伸猫的身体。

本层请求样例：

```json
{
  "model": "seedream-5.5",
  "prompt": "请以参考图中的猫为主角，保留其毛色分布、脸部花纹、眼睛颜色和粉色鼻子。让它坐在明亮的摄影棚里，头颈清晰可见，完整保留双耳，背景为柔和浅蓝色，无其他动物，无文字，无水印。写实宠物摄影，柔和自然光，毛发细节清晰。根据请求的画布比例自然调整留白，不要拉伸猫的身体。",
  "size": "2K",
  "output_resolution": "2K",
  "aspect_ratio": "1:1",
  "duration": 1,
  "response_format": "url",
  "image": [
    "https://images.unsplash.com/photo-1514888286974-6c03e2ca1dba?w=1024&q=85&fit=crop"
  ]
}
```

上游使用改写后的模型名和统一的 `images` 字段，其余参数保留。验证时解包返回 URL，直接下载天悦原图，以图像解码结果检查格式、像素和比例；不做缩放、裁切或压缩。公共代理前缀使用本地测试地址，仅检查 URL 包装结果，未验证线上外部图片代理服务。

## 九种比例的实测结果

所有请求都带同一张参考图，分辨率参数均为 `2K`。以下耗时为本层同步生成接口的响应时间，不含下载和图片解码。

| 请求比例 | 参考图输入字段 | 生成 HTTP | 图片下载 HTTP | 实际像素 | 生成耗时 |
| --- | --- | --- | --- | --- | --- |
| `9:21` | `image` | 400 | 无图片 | 无图片 | 0.24 秒 |
| `9:16` | `reference_images` | 200 | 200 | 1600 × 2848 | 60.74 秒 |
| `2:3` | `reference_images` | 200 | 200 | 1664 × 2496 | 51.28 秒 |
| `3:4` | `image` | 200 | 200 | 1728 × 2304 | 60.73 秒 |
| `1:1` | `image` | 200 | 200 | 2048 × 2048 | 40.56 秒 |
| `4:3` | `reference_images` | 200 | 200 | 2304 × 1728 | 41.63 秒 |
| `3:2` | `image` | 200 | 200 | 2496 × 1664 | 60.50 秒 |
| `16:9` | `images` | 200 | 200 | 2848 × 1600 | 50.93 秒 |
| `21:9` | `reference_images` | 200 | 200 | 3136 × 1344 | 60.47 秒 |

八张成功结果均为 PNG，返回体包含 `created`、`data`、`usage`，图片原始 host 为 `ddw2.tianyue.xyz`，均按既有天悦 URL 规则包装。三种参考图别名都取得了成功生成结果。

`16:9` 与 `9:16` 的实际宽高比相对请求值有约 0.125% 的偏差，其余六种成功比例与请求比值精确相等。自动检查的相对误差阈值为 1%；报告保留实际像素，便于独立判断。

`9:21` 的上游响应如下，模型名已是正确的上游 ID：

```json
{
  "error": {
    "message": "不支持的比例参数：9:21，模型: GZ-seedream-5.5-2K",
    "type": "upstream_error",
    "param": "",
    "code": "invalid_aspect_ratio"
  }
}
```

本次共提交九次同步生成请求；最初三组成功结果未重复生成。没有提交无参考图请求，没有使用异步任务接口。

## 图像检查与本地产物

对照图包含参考图、八张生成图和 `9:21` 的失败标记。目视检查可见生成图保留了黑白毛色、绿色眼睛、粉色鼻子和完整双耳，背景变为浅蓝色。不同画幅的留白和姿态有变化，不能据此宣称精确身份还原。

- [完整对照图](/tmp/tianyue-seedream55-QPXk1Q/contact-sheet.jpg)
- [逐次结果与原图 SHA-256](/tmp/tianyue-seedream55-QPXk1Q/results.json)
- 参考图和八张未修改的上游 PNG 保留在 `/tmp/tianyue-seedream55-QPXk1Q/`。

代码改动尚未推送或部署；本报告验证的是当前本地构建到真实天悦上游的链路。

## 16:1 补充验证（2026-11-12）

使用 `AIwork_Personal` 当前 Seedream 5 Pro 的请求体结构测试 `16:1`：`model=seedream-5.5`、`size=2K`、`aspect_ratio=16:1`、`n=1`、`watermark=false`、`response_format=url`，参考图使用 `image` 数组，提示词以 `,16:1` 结尾。请求经过本地代理并发往天悦；本次没有重发已通过的其他比例。

上游拒绝该请求，未返回图片：

```json
{
  "error": {
    "message": "不支持的比例参数：16:1，模型: GZ-seedream-5.5-2K",
    "type": "upstream_error",
    "param": "",
    "code": "invalid_aspect_ratio"
  }
}
```

这验证了本层模型映射和客户端请求字段均已到达天悦，但 `GZ-seedream-5.5-2K` 在该渠道不接受 `16:1`，即使其他官方 API 支持此比例；不能把对官方 API 的支持外推为此渠道可用。

## 1:16 补充验证（2026-11-12）

沿用 `16:1` 的同一请求体、`2K`、`image` 参考图和提示词格式，仅将 `aspect_ratio` 及提示词后缀改为 `1:16`。天悦以 HTTP 400 拒绝：

```json
{
  "error": {
    "message": "不支持的比例参数：1:16，模型: GZ-seedream-5.5-2K",
    "type": "upstream_error",
    "param": "",
    "code": "invalid_aspect_ratio"
  }
}
```

未生成图片。该比例与 `16:1` 一样不受此渠道的 `GZ-seedream-5.5-2K` 支持。
