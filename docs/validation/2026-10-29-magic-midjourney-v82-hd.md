# Magic V8.2 标准与高清生成对照

测试日期：2026-10-29（Asia/Shanghai）。使用现有 Magic 凭据和 [midjourney_smoke.py](../../scripts/midjourney_smoke.py) 直连原生异步接口；本次未经过本地 OpenAI 同步代理，未修改产品代码或默认参数。

## 结论

`mj_fast_imagine`、V8.2 可通过提示词中的 `--hd` 生成更高像素的候选单图。1:1 对照中，`--sd` 单图为 1024×1024，`--hd` 单图为 2048×2048，即宽高各两倍、总像素四倍。本次没有调用生成后的 Upscale 接口。

| 参数 | 任务 ID | 状态 | 候选单图 | 四宫格主图 | 耗时 |
| --- | --- | --- | --- | --- | --- |
| `--sd` | `1793245652521105` | `SUCCESS` | 1024×1024 PNG | 2048×2048 PNG | 339.61 秒 |
| `--hd` | `1793245642855655` | `SUCCESS` | 2048×2048 PNG | 2048×2048 PNG | 392.26 秒 |

耗时从提交前开始，包含轮询与四宫格主图下载，不包含后续候选单图下载。两个任务在 95% 停留较久；单次耗时不代表通常延迟，也不用于推算渠道费用。

**本渠道高清任务的四宫格主图仍是 2048×2048，应读取 `imageUrls` 中的候选单图链接获取高清结果。** 仅检查 `imageUrl` 尺寸会误判高清没有生效。当前代理的结果选择规则已优先采用候选单图列表，见 [FAST 调用说明](../magic-fast-openai.md)。

## 请求

两组均调用 `POST https://magic666.top/mj/submit/imagine`，`mode=FAST`。使用相同中文正文、比例、版本、风格化与 seed，仅切换 `--sd` / `--hd`；固定 seed 用于减少对照变量，不承诺两个分辨率结果逐像素一致。

高清请求：

```json
{
  "mode": "FAST",
  "prompt": "一只橙色陶瓷狐狸，戴着蓝色针织围巾，端坐在原木桌面上，奶油色背景，温暖柔和的摄影棚灯光，主体完整入镜，无文字，无水印 --ar 1:1 --v 8.2 --s 100 --seed 246810 --hd --fast"
}
```

标准对照只把 `--hd` 改为 `--sd`。提交后使用 `GET /mj/task/{taskId}/fetch` 查询，两个任务均有明确的 `SUCCESS` 终态。

高清任务的 `properties.finalPrompt`：

```text
一只橙色陶瓷狐狸，戴着蓝色针织围巾，端坐在原木桌面上，奶油色背景，温暖柔和的摄影棚灯光，主体完整入镜，无文字，无水印 --ar 1:1 --v 8.2 --hd --seed 246810 --stylize 100 --fast
```

标准任务的最终参数包含 `--sd --ar 1:1 --v 8.2 --seed 246810 --stylize 100 --fast`。两组各返回四个 `imageUrls`。

两组共八张候选单图和两张主图均已下载，用 Pillow 完整解码后读取尺寸。高清四张单图均为 2048×2048，文件分别为 4,633,928、5,491,870、4,607,143、4,902,366 字节；标准四张均为 1024×1024，文件分别为 1,099,321、1,320,369、1,378,469、1,199,722 字节。文件未作本地缩放。

## 结果链接与复核

- [高清候选单图 1](https://imagine-cdn.youchuan.cn/50ae7195-40a3-4578-bd3e-5c1a1083e6d3_0_0.png)。
- [标准候选单图 1](https://imagine-cdn.youchuan.cn/be70613e-ef23-446d-aa34-562e51f8cac7_0_0.png)。
- [高清任务四宫格](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1793245642855655_bff6e15691014c12b9ba6b778a3b6014.png)。
- [标准任务四宫格](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1793245652521105_45327498f4eb483bbb850ef7fc0ae329.png)。

本机原始材料位于 `/tmp/mj-v82-hd-20261029-qKbkVH/hd/` 和 `sd/`：请求、提交响应、最终任务记录、摘要、四宫格和候选单图。凭据通过隐藏输入提供，不写入请求记录或仓库。

本次验证范围为 FAST、V8.2、1:1 文生图；未据此声称 RELAX、参考图、其他比例或 Niji V7 的高清组合已经通过。

官方参数列表将 `--hd` / `--sd` 说明为高清 2048px / 标准 1024px：[Parameter List](https://docs.midjourney.com/hc/en-us/articles/32859204029709-Parameter-List)。本次读取同站官方 JSON 的 `article.body` 核对语法，实际渠道结论以任务和下载图片为准。
