# Magic Midjourney V8.2 渠道实测

测试日期：2026-10-22（Asia/Shanghai）。本次使用现有凭据直连 `https://magic666.top` 的原生异步接口，中文提示词直接发送；不经过本地 `/v1/images/generations` 代理。未修改代码、模型标识或默认版本。

## 结果

| 模型／场景 | 结果 | 任务 ID | 耗时 |
| --- | --- | --- | --- |
| `mj_fast_imagine` 中文文生图 | `SUCCESS`，图片已下载并完整解码 | `1792632926621263` | 100.25 秒，包含主图下载 |
| `mj_fast_imagine` 中文 + 单张 Base64 参考图 | `SUCCESS`，图片已下载并完整解码 | `1792633037903974` | 110.09 秒，包含主图下载 |
| `mj_relax_imagine` 中文 + 单张 Base64 参考图 | 提交业务 code `3`，`No available account instance`，未取得任务 ID | 无 | 未进入生成 |

FAST 文生图和单张 Base64 参考图均已成功接受并完成 `--v 8.2` 任务，最终提示词也保留该版本。RELAX 本次因无可用账号实例未完成验证，不能据此认定支持或不支持 8.2。多参考图与其他参数组合不在本次验证范围内。

## 请求与版本证据

FAST 提交：`POST /mj/submit/imagine`。

```json
{
  "mode": "FAST",
  "prompt": "一只橙色陶瓷狐狸，戴着蓝色针织围巾，端坐在原木桌面上，奶油色背景，温暖柔和的摄影棚灯光，主体完整入镜，无文字，无水印 --ar 1:1 --v 8.2 --fast"
}
```

通过 `GET /mj/task/1792632926621263/fetch` 取得 `status=SUCCESS`。任务的 `properties.finalPrompt` 为：

```text
一只橙色陶瓷狐狸，戴着蓝色针织围巾，端坐在原木桌面上，奶油色背景，温暖柔和的摄影棚灯光，主体完整入镜，无文字，无水印 --ar 1:1 --v 8.2 --stylize 100 --fast
```

上游追加了 `--stylize 100`，版本保留 `8.2`。这里以 API 接受、最终参数、生成终态和图片下载作为渠道可用性证据，未独立检验服务端实际运行的模型权重。

主图为 2048×2048 PNG 四宫格，8,039,530 字节；返回四张候选单图链接。第一张单图已下载、完整解码并目视检查，为 1024×1024 PNG，包含橙色狐狸和蓝色针织围巾。四宫格尺寸不等于每张候选单图尺寸。

- [文生图四宫格](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792632926621263_003f39431cff44de837a80907fb9eb5f.png)
- [文生图候选单图 1](https://imagine-cdn.youchuan.cn/0b6a08f6-dac5-4a01-82c9-08c735c2cf29_0_0.png)

## Base64 参考图

FAST 参考图请求使用同一提交路径与 `mode=FAST`，增加单元素 `base64Array`，元素为 `data:image/jpeg;base64,` 加原始 JPEG 的 Base64 编码。输入是既有 512×512 JPEG，38,983 字节。

提示词直接发送：

```text
请参考图片中的橙色陶瓷狐狸，保留蓝色针织围巾和陶瓷质感，让它坐在雨天的窗台上，窗外有朦胧雨滴，柔和自然光，单个主体完整入镜，无文字，无水印 --ar 1:1 --v 8.2 --fast
```

任务 `1792633037903974` 的最终提示词在正文前增加上游物化的参考图 URL，后缀为 `--ar 1:1 --v 8.2 --stylize 100 --fast`。状态为 `SUCCESS`，返回四张候选单图链接，主图下载后为 2048×2048 PNG 四宫格，9,589,646 字节。目视检查可见橙色狐狸、蓝围巾以及雨天窗户背景；部分候选为近景，未完全遵循完整入镜要求。

[参考图生成结果](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792633037903974_d527b2e5ea3641348fa8e8273389af26.png)

## RELAX 容量限制

RELAX 请求使用 `POST /mj-relax/mj/submit/imagine`、`mode=RELAX`、提示词后缀 `--ar 1:1 --v 8.2 --relax` 和单元素 `base64Array`，图片为同一张 512×512 JPEG。提交返回：

```json
{
  "code": 3,
  "description": "No available account instance",
  "result": null,
  "properties": {}
}
```

## 复核材料

使用 [现有探针](../../scripts/midjourney_smoke.py)，通过隐藏输入提供 key，未将 key 写入仓库或结果文件。

本机原始证据位于 `/tmp/mj-v82-smoke-20261022-7iMeYw/`，包含各场景的请求、提交响应、最终任务记录，以及成功场景的主图与摘要。Base64 图片载荷仅保存在本机测试目录，不复制到本文。该目录属于临时存储；长期结论以本文记录为准。
