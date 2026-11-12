# Midjourney 参数产品化调研：现有 Magic / V8.1 渠道

后续决策更新（2026-10-22）：用户已确认渠道支持 V8.2，产品范围改为“Midjourney V8.2 主模型 + 最新 Niji 动漫模型”。当前规划以[双模型控件规划](../plans/2026-10-22-midjourney-v82-niji7-controls.md)为准；本文保留 V8.1 参数调研及历史建议，不再作为当前产品版本选择依据。

参数整理口径更新：当前规划直接采用官方明确的支持情况、范围和默认值。本文保留的渠道验证建议属于历史调研过程，不作为当前参数控件的启用条件。

调研日期：2026-10-22。本文给后续产品和接口开发作参考，原始资料调研未新增计费任务，不变更现有 API、前端或生成行为。用户随后要求的 [V8.2 渠道实测](../validation/2026-10-22-magic-midjourney-v82.md)已完成：FAST 中文文生图和单张 Base64 参考图成功；RELAX 因无可用账号实例未完成验证。以下参数表仍以 V8.1 为基线，不视为 V8.2 全参数验收。

配套资料：[官方参数、范围及版本兼容表](2026-10-22-midjourney-official-parameters.md)。以下“建议”是产品设计判断；官方事实、渠道文档与本地实现证据分别列出，不能互相替代。

## 1. 结论与版本基线

推荐让用户只填写画面描述，把比例、风格化、变化度、Raw、排除元素和普通参考图做成独立选项；程序统一生成末尾的 `--` 参数。官方本来就区分文字描述、图片输入和生成参数，要求参数放在提示词最后并与正文用空格分隔。[O1]、[O2]

官方当前默认模型已经是 V8.2，发布公告日期为 2026-07-24；本项目现有渠道与历史测试锁定的是 `--v 8.1`。因此产品版本建议明确固定为 8.1，等渠道验证完成后再增加其他版本，避免省略版本造成模型漂移。`mj_fast_imagine` / `mj_relax_imagine` 是本项目的渠道模型标识，`--v 8.1` 才是 Midjourney 生成版本，两者应分开存储。[O3]、[O4]、[L1]、[L2]

界面能力应由“Midjourney 版本 × Magic 渠道 × 当前接入方式”共同决定。官方支持某参数，不等于 Magic 已支持所有取值或组合；当前 Magic 文档描述 Imagine 的 prompt / base64Array 协议，没有逐版本、逐参数的支持矩阵。[M1]、[M2]

## 2. 首版界面建议

下表是 V8.1 的候选控件。默认值和合法范围来自官方，是否向产品用户启用仍需对应 Magic 渠道验证。滑块的友好预设是产品建议，不能当成官方推荐档位。[O3]、[O5]

| 界面选项 | 编译结果 | 官方 V8.1 范围／默认 | 产品建议 |
| --- | --- | --- | --- |
| 版本 | `--v 8.1` | 当前项目选择 8.1；官网默认已变为 8.2 | 首版固定，展示为版本信息；不要自动跟随官网最新版本 |
| 画面比例 | `--ar 3:2` | 默认 1:1；整数宽高比。SD 最大 14:1，HD 最大 4:1，均含反向比例 | 提供 1:1、3:2、2:3、4:3、3:4、16:9、9:16，首版不开放极端比例 |
| 风格化程度 | `--s 100` | 0–1000，默认 100 | 默认 100；值越高，模型美学介入越强，不等同提示词遵循度或输出质量 |
| 变化度 | `--c 0` | 0–100，默认 0 | 首屏可折叠；解释为一组候选的差异程度，不是图片数量 |
| Raw | `--raw` | 开关参数，省略时可能受账号设置影响 | 产品可期望默认关闭，但须核对渠道账号默认；减少模型自动美学加工，不命名为“无风格” |
| 排除元素 | `--no text, watermark` | 排除元素参数 | 独立标签输入；不自动插入“低质量”等无依据的通用负面词库，也不承诺绝对消除元素 |
| 生成速度 | 不要求前端拼接模式参数 | 本项目 model 决定 FAST / RELAX | 选“快速／慢速”后映射两个已有 model；不提供当前不支持的 Turbo |
| 普通参考图 | 请求 `image: [URL, ...]` | 官方 Image Prompt 影响内容与视觉；本代理最多六张 | 独立上传区，标注“内容／构图参考”；本层六张限制不是上游六张实测承诺 |
| 普通参考强度 | `--iw 1` | 0–3，默认 1 | 仅存在普通参考图时显示；一个全局权重，不默认提供每张独立权重 |

比例、风格化、变化度、Raw、No、图像权重的原始说明及兼容依据见配套官方表和其逐项链接。SD/HD 比例上限来自官方 Version 兼容表，不能把 SD 的 14:1 上限同时用于 HD；Aspect Ratio 专页补充整数比例语法。[O3]、[O5]、[O6]

`--no` 的多词短语不应当成不可拆的整体否定。官方例子 `--no modern clothing` 会分别排除 modern 和 clothing，可能产生非预期结果；此时更适合在正文正向描述想要的衣服。排除区建议用简短元素标签。[O5]

上述参数中，项目既有原生 Magic 测试覆盖 `--v 8.1`、`--ar 1:1` / `3:2`、`--iw 1.5`、FAST / RELAX 等若干组合；并没有覆盖所有参数及其边界。新增 OpenAI 代理链路的本地测试与真实生成记录须分别看待。[L2]、[L3]、[L4]

## 3. 放入高级选项的能力

| 能力 | 参数 | 已核实的事实 | 产品建议 |
| --- | --- | --- | --- |
| 风格参考 | `--sref URL`、`--sw 100` | V8.1 支持；`--sw` 0–1000，默认 100；参考整体视觉风格，不等于精确保留人物身份 | 与普通参考图分区，后续单独验证 Magic URL 可达性、权重及多图组合 |
| 风格参考算法版本 | `--sv` | 详细文档给出的 V6 / V7 枚举与默认值不能直接当作 V8.1 合同 | 当前产品隐藏，不把 `--sv 6` 硬编码成 V8.1 默认 |
| 特异程度 | `--w 0` | V8.1 支持；0–3000，默认 0 | 高级滑块；不与“变化度”合并，它控制的是更奇特的视觉倾向 |
| 实验美学 | `--exp 0` | V8.1 兼容；官方参数介绍范围 0–100、默认 0 | 高级实验项，先补渠道验证；不承诺与旧版本相同的数值视觉效果 |
| 种子 | `--seed 12345` | 整数 0–4294967295，默认随机；不是人物身份锁定工具 | 默认随机，不发送 seed；高级用户可指定，历史记录保存实际输入，不承诺跨版本／运行条件精确复现 |
| 分辨率 | `--sd` / `--hd` | V8.1 有标准与高清生成；官方分别标为 1024px / 2048px，HD 可用比例更窄 | 经渠道实测后提供“标准／高清”；不要套用 OpenAI quality 字段或旧 Upscale 接口 |
| 无缝纹理 | `--tile` | V8.1 支持；官方仍持续更新拼接行为，附有 Edit 输入时有兼容限制 | 做为纹理场景的高级开关；验收实际边缘拼接，而不仅检查返回成功 |
| 个性化 | `--p` / `--profile` | 关联 Midjourney 账号的个性化配置和 moodboard | 需要先确认渠道账号及 profile 可用性，当前不作为通用风格预设 |

以上官方事实的逐项来源、V8.1/8.2 与旧版差异见[配套官方表](2026-10-22-midjourney-official-parameters.md)。风格参考专页还区分参考 URL、风格代码及版本行为，因此不能用同一个“图片参考强度”滑块代替 `--iw` 与 `--sw`。[O7]

V8.1 发布公告曾改变 HD 默认行为，但网站账号设置和 Magic 渠道默认不能据此推定。若产品对输出分辨率有明确承诺，建议在渠道验证后显式发送 `--sd` 或 `--hd`，并检查每张候选单图尺寸。历史记录的 2048×2048 主图是四宫格，而其中单图曾为 1024×1024；主图尺寸不能当成每张候选的分辨率。[O8]、[L2]

官方 Relax 限制中包含 HD 视频、repeat 和 permutations，并未把静态 HD 图片整体列为禁用。因此不能从视频限制推断图片 `--hd --relax` 不可用，也不能反过来宣称 Magic 已验证此组合。[O5]

## 4. 当前界面不应照搬的旧参数和网站功能

| 项目 | 原因与处理建议 |
| --- | --- |
| `--q` / `--quality` | 官方版本矩阵列 V8.1 不支持。不能按 V6/V7 教程为 8.1 添加质量档位；改为单独研究 SD/HD。 |
| `--stop` | 属于旧版本能力，不是当前 V8.1 通用的生成进度控制。 |
| `--cref` / `--cw` | 旧 Character Reference 属于 MJ/Niji 6；不应该直接用于 V8.1 的角色一致性功能。 |
| `--oref` / `--ow` | Omni Reference 属于 V7；V8.X 改为 Edit Model 路径，不能只给 8.1 加一个主体参考图参数便宣称支持。 |
| 文本多段 `::` 权重 | V8.1 不支持旧式 multi-prompt 文本权重；不要照搬 V6 教程。它与普通图片权重 `--iw` 不是同一机制。 |
| `--turbo` | V8.1 官方矩阵不支持，且当前代理会按 model 把它规范化为 FAST / RELAX。 |
| `--draft` | 官方资料存在版本更新不一致：新版详细页及更新公告描述 V8.1/8.2 网页的 24 张草稿，旧总览／矩阵仍写 V7 或不支持。网页新能力不能据此认定当前 Magic 的单次 Imagine 链路支持。 |
| `--repeat` / `--r`、permutations | 批量产生多组任务，需单独处理任务数、额度、限流与同步接口合同；不等同当前单任务返回的四张候选。 |
| `--stealth` / `--public` | 是 Midjourney 账号及其网站可见性行为，不能据此承诺第三方 Magic 或本产品存储的图片隐私。 |
| `--niji` | 切换另一模型家族，不能仅作为 V8.1 的“动漫风格”开关；需要单独版本和渠道能力判断。 |
| Thinking、网站 Rerun 等 | 最新网页功能未必有公开可提交参数；没有可核实的 `--thinking` 语法时不自行发明。 |
| `--edit`、放大、变体、视频 | Edit 要区分参考图角色和输入约束；放大、变体、视频还涉及各自任务流程，不能仅靠添加几个开关就当作已接入。 |

旧参数的兼容性依据为官方 Version、Parameter List 及逐项文档；Draft 和最新 Edit/Tile 变化另以更新公告补足，并保留官方资料冲突说明。[O2]、[O3]、[O9]、[O10]

## 5. 普通参考、风格参考和主体编辑应分开

| 用户意图 | Midjourney 机制 | 本项目当前接入事实 |
| --- | --- | --- |
| 参考内容、构图、配色，重新生成画面 | Image Prompt；`--iw` 调整整体图像影响 | 已有 image URL 数组 → 下载／预处理 → base64Array。 |
| 参考画风、质感与视觉语言 | Style Reference；`--sref`、`--sw` | 普通 image 数组不会自动变成风格参考；手写 `--sref URL` 只随 prompt 透传，当前不会下载该 URL 并作为专门风格图物化。 |
| 保持同一人物／物体，或依照图片修改 | V7 Omni 与 V8.X Edit 的不同机制；V8 使用 `--edit URL1 URL2`，官方最多四张图 | 当前没有分离的主体参考／Edit 请求合同；普通 image 数组不能自动视为 Edit 输入，需要单独验证 Magic 支持。[O11] |

项目历史原生 Magic `--sref URL` 成功记录可作为有限渠道依据，且目视风格接近；它没有量化身份或风格一致性，也没有证明多图风格参考、所有 `--sw` 取值或代理专用风格图处理已经跑通。[L2]

图片输入缩到最长边 2048、JPEG 质量 98、4:4:4，是本项目的参考图预处理规则；它们不是生成参数，也不是输出高清档位。[L1]、[L3]

## 6. 前端如何生成当前接口的请求

建议前端分别保存 `promptText` 与结构化界面选项，再由一个参数编译器拼出最终 prompt。后端仍校验最终请求；不要让多个控件、浏览器保存值和高级手写参数各自覆盖同一选项。

例如，用户只输入：

> 一只橙色陶瓷狐狸，戴蓝色针织围巾，坐在原木桌面上，奶油色背景，温暖柔和的摄影棚灯光

界面选择：V8.1、3:2、风格化 100、变化度 0、Raw 开启、排除文字和水印、快速模式。基于当前 API 合同，客户端可以构造以下请求；这是拼接示例，并非本轮真实生成实测：

```json
{
  "model": "mj_fast_imagine",
  "prompt": "一只橙色陶瓷狐狸，戴蓝色针织围巾，坐在原木桌面上，奶油色背景，温暖柔和的摄影棚灯光 --v 8.1 --ar 3:2 --s 100 --c 0 --raw --no text, watermark",
  "response_format": "url"
}
```

普通参考图则增加现有 `image` URL 数组，并在有需要时追加一个 `--iw`。选择慢速只需换 `model`，模式参数由代理统一管理。首版可显式发送 UI 的确定默认值；遇到未知或账号相关默认项，使用“自动”并省略参数，不臆造数值。

当前后端不会把新增 JSON 字段自动翻译成 MJ 参数；尤其顶层 `quality`、`size`、`n`、`stream` 被明确拒绝。UI 中的“风格化／分辨率／参考强度”只是客户端状态，应编译进 prompt；若未来要开放结构化后端参数，需要另行定义 API 合同。[L1]

参数编译器的建议规则：

1. 正文在前，参数在后，使用 ASCII `--` 和明确空格；不把句号或中文逗号追加在数值／开关结尾。`--ar` 的冒号、`--no` 允许的分隔形式等按对应参数语法处理，不能全局删除所有标点。[O2]
2. 同一设置只生成一次：统一 `--aspect` / `--ar`、`--stylize` / `--s` 等别名，控制数值范围和互斥组合，例如 SD/HD 及 HD 比例上限。[O2]、[O5]
3. 若开放高级手写参数，先解析并提示与界面选项的冲突；不要假定任意重复参数均按“最后一个生效”。现有代理仅规范化三种速度 token，其余参数没有为 UI 做完整冲突管理。[L1]
4. 保存用户原文、结构化选项、最终请求 prompt、model 和版本，便于历史回填与排障；随机种子与任务实际输出另行记录，不能把本层固定 usage 当真实计费。
5. 模型版本切换时重新校验参考图角色及参数组合，不能把旧版本所有参数原样带入新版本。

开关的“关闭”还需区分省略参数和实际生效状态：`--raw false` 不是本文已确认的关闭语法。遇到由共享渠道账号继承的 Raw、个性化等设置，产品先确认可控的基线；在不能确保关闭时使用“自动／继承渠道设置”等诚实表述，而不是省略参数却承诺已强制关闭。[O5]

## 7. 后续开发前建议补的最小渠道验证

本节是调研结论产生的候选验证计划，本轮未执行。每项结果建议区分：被接受、进入最终任务参数、生成完成、视觉／尺寸效果符合预期；仅“HTTP 200”不足以证明参数有效。

| 验证组 | 观察目标 |
| --- | --- |
| 固定 V8.1，常用横竖比例 | 最终版本、每张候选尺寸及实际比例；不要只看四宫格主图 |
| `--s`、`--c`、`--raw`、`--no` | 低／默认／较高取值的接受情况，以及视觉差异；主张风格倾向而非绝对保证 |
| 单图／双图 + `--iw` | 参数最终保留，普通参考图数量、顺序和权重行为；两种 model 的完整代理链路 |
| `--sref URL` + `--sw` | 风格图能被上游读取，角色与普通 image 数组区分；必要时再调查可靠的风格图上传路径 |
| `--sd` / `--hd` + 两种速度 | 单候选分辨率、时延、费用来源和比例限制，先明确 Magic 实际支持再启用 UI |
| 官方不支持或冲突的参数 | 产品在提交前解释不兼容项，避免依靠上游静默忽略 |

不为本次调研自动切换 V8.2，不添加新渠道模型，也不把官方新网页能力视为已接入。

## 来源

### Midjourney 官方

- [O1] [Prompt Basics](https://docs.midjourney.com/hc/en-us/articles/32023408776205-Prompt-Basics)，页面 updated_at 为 2026-08-31。
- [O2] [Parameter List](https://docs.midjourney.com/hc/en-us/articles/32859204029709-Parameter-List)，页面 updated_at 为 2026-09-01。
- [O3] [Version](https://docs.midjourney.com/hc/en-us/articles/32199405667853-Version)，当前版本及逐版本功能矩阵。
- [O4] [Version 8.2 发布公告](https://updates.midjourney.com/version-8-2/)，2026-07-24。
- [O5] [官方逐参数范围和兼容性摘录](2026-10-22-midjourney-official-parameters.md)，包含每项的官方专页链接及访问情况。
- [O6] [Aspect Ratio](https://docs.midjourney.com/hc/en-us/articles/31894244298125-Aspect-Ratio)。
- [O7] [Style Reference](https://docs.midjourney.com/hc/en-us/articles/32180011136653-Style-Reference)。
- [O8] [V8.1 Alpha 发布公告](https://updates.midjourney.com/v8-1-alpha/)，2026-04-14。
- [O9] [Alpha Changelog 10/7/26](https://updates.midjourney.com/alpha-changelog-10-7-26/)，页面发布于 2026-10-08，补充 Draft / Thinking / Tile 的最新变化。
- [O10] [Edit Updates, Thumbnail Previews, and More](https://updates.midjourney.com/edit-updates-thumbnail-previews-and-more/)，2026-09-24。
- [O11] [Edit Model](https://docs.midjourney.com/hc/en-us/articles/48495453462797-Edit-Model)，页面 updated_at 为 2026-09-24。

官方帮助中心 HTML 如遇 Cloudflare 验证，可读取相同文章的官方 JSON：`https://docs.midjourney.com/api/v2/help_center/en-us/articles/{articleId}.json`，其中 article.body、html_url、updated_at 可供核对；来源仍是 Midjourney 官方，不采用第三方转载填补参数值。

### Magic 官方

- [M1] [Midjourney 概览](https://docs.magic666.top/api-reference/images/midjourney/overview.md)。
- [M2] [任务提交](https://docs.magic666.top/api-reference/images/midjourney/submit.md)。
- [M3] [任务查询](https://docs.magic666.top/api-reference/images/midjourney/query.md)。

### 本项目证据

- [L1] [MJ 请求解析和模式规范化](../../src/midjourney.rs)：Mode、prepare_request、normalize_prompt；读取当前工作区实现，未修改。
- [L2] [2026-10-21 原生 Magic / v8.1 实测](../validation/2026-10-21-midjourney-v81.md)：仅使用其记录时的事实，不将旧文中的“尚未实现代理”当作当前状态。
- [L3] [当前参考图调用说明](../magic-reference-openai.md)：普通参考图、预处理、输出及接口限制。
- [L4] [2026-10-22 双模型多参考图验收记录](../validation/2026-10-22-magic-multiple-references-openai.md)：本地模拟验证与尚未执行的真实代理验收分别记录。

[O1]: https://docs.midjourney.com/hc/en-us/articles/32023408776205-Prompt-Basics
[O2]: https://docs.midjourney.com/hc/en-us/articles/32859204029709-Parameter-List
[O3]: https://docs.midjourney.com/hc/en-us/articles/32199405667853-Version
[O4]: https://updates.midjourney.com/version-8-2/
[O5]: 2026-10-22-midjourney-official-parameters.md
[O6]: https://docs.midjourney.com/hc/en-us/articles/31894244298125-Aspect-Ratio
[O7]: https://docs.midjourney.com/hc/en-us/articles/32180011136653-Style-Reference
[O8]: https://updates.midjourney.com/v8-1-alpha/
[O9]: https://updates.midjourney.com/alpha-changelog-10-7-26/
[O10]: https://updates.midjourney.com/edit-updates-thumbnail-previews-and-more/
[O11]: https://docs.midjourney.com/hc/en-us/articles/48495453462797-Edit-Model
[M1]: https://docs.magic666.top/api-reference/images/midjourney/overview.md
[M2]: https://docs.magic666.top/api-reference/images/midjourney/submit.md
[M3]: https://docs.magic666.top/api-reference/images/midjourney/query.md
[L1]: ../../src/midjourney.rs
[L2]: ../validation/2026-10-21-midjourney-v81.md
[L3]: ../magic-reference-openai.md
[L4]: ../validation/2026-10-22-magic-multiple-references-openai.md
