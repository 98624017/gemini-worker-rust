# Midjourney 官方参数调研：V8.1 与当前版本

后续产品决策（2026-10-22）：用户已确认渠道支持 V8.2，当前目标改为 V8.2 + 最新 Niji。本文的 V8.1 参数字典保留作历史参考；当前产品范围见[双模型控件规划](../plans/2026-10-22-midjourney-v82-niji7-controls.md)，Niji 兼容性见[Niji 官方参数调研](2026-10-22-niji7-official-parameters.md)。

参数整理口径更新：当前规划直接采用官方文档明确的参数功能、范围和默认值；本文历史渠道验证建议不作为当前控件的启用条件。

调研日期：2026-10-22（Asia/Shanghai）。范围：Midjourney 官方文档和官方更新公告；不调用生成接口，不修改产品代码。本文用于后续参数控件设计，官方支持不等于 Magic 渠道已支持。

## 先读这几条

- 当前官方默认版本已经是 **V8.2**，2026-07-24 发布为默认；V8.1 于 2026-04-14 发布，2026-06-10 至 07-23 曾是默认。当前项目固定 `--v 8.1` 时应按 V8.1 能力校验，不能套用旧 V6 教程或把“官方最新”自动替换到渠道。[版本]、[V8.1公告]、[V8.2公告]
- V8.1 常见可用项是 `--ar`、`--s`、`--c`、`--w`、`--exp`、`--seed`、`--no`、`--raw`、`--tile`、`--iw`、`--sref`、`--sw`、`--sd/--hd`、`--fast/--relax`。`--q`、`--stop`、`--cref/--cw`、`--oref/--ow` 不应作为 V8.1 控件提供；`--turbo` 也不兼容 V8.1。[版本]、[遗留功能]、[GPU速度]
- 图片参考有不同角色：普通 Image Prompt 参考内容/构图/颜色；Style Reference 参考风格；V8 的 Edit Model 用于指令编辑和人物/物体保留。它们不能只用一个“参考图强度”混为一谈。[图像提示]、[风格参考]、[编辑模型]
- `--hd` 是 V8.1 原生高分辨率生成，不是生成后 Upscale。SD/HD 影响成本、尺寸和最大比例；不应把 `--q` 当作 V8.1 高清选项。[版本]、[画幅]、[质量]
- 官方页面存在陈旧段落和互相冲突的表格，尤其 Draft；下文明确列出证据，不把未知值补成默认值。

## 参数语法

官方要求参数放在正文后面，正文与 `--` 之间留空格，两个短横线不能拆开；参数后不要再接正文。`--ar 16:9` 这种值紧跟参数；`--raw`、`--tile` 这类开关不接布尔值。[参数总览]

例如：

```text
一只橙色陶瓷狐狸，蓝色围巾，奶油色背景 --ar 1:1 --v 8.1 --s 100 --raw --sd --fast
```

总览的“参数不要有标点”指语法尾部不要乱加标点，不应误读成禁止值中的所有标点：官方明确用 `--ar 16:9`，且 `--no fruit, apple, pear` 允许逗号分隔负向元素。[参数总览]、[画幅]、[排除]

`--no` 不会可靠地把多词短语当成一个不可拆的概念。官方例子 `--no modern clothing` 会被分别理解成排除 modern、排除 clothing，可能出现非预期结果；此时宜在正文正向描述想要的衣服。产品的排除输入可使用简短元素标签，不应宣传为自然语言精确反向编辑。[排除]

## V8.1 参数字典

下表“支持”是官方能力；是否经 Magic 稳定透传、是否存在上游账户限制，须另行验证。

| 功能 | 规范语法 / 别名 | 官方默认与取值 | V8.1 状态与解释 | 来源 |
| --- | --- | --- | --- | --- |
| 画幅 | `--ar W:H` / `--aspect W:H` | 默认 `1:1`；整数比，不能用小数；可用 `85:110` 表示 8.5:11 | 支持；SD 最宽/最高 14:1（或 1:14），HD 为 4:1（或 1:4）；比例不是指定像素 | [画幅]、[版本] |
| 风格化 | `--s N` / `--stylize N` | 默认 100；0–1000 | 支持；低值更贴文字，高值更偏模型审美，不是“画质” | [风格化]、[版本] |
| 多样性 | `--c N` / `--chaos N` | 默认 0；0–100 | 支持；提高同组候选差异，可能减弱提示贴合度；网页称 Variety | [多样性]、[版本] |
| 怪诞 | `--w N` / `--weird N` | 默认 0；0–3000 | 支持；实验特性，与 seed 并非完全兼容；`--w` 这里不是宽度 | [怪诞]、[版本] |
| 实验审美 | `--exp N` | 默认 0；0–100；公告建议 5/10/25/50/100 | 支持（版本表确认）；范围来自原始官方公告。可能增加细节/动态/创意，也可能牺牲文字准确度和多样性；25–50 以上可能压过 stylize/personalization | [实验参数]、[版本] |
| 随机种子 | `--seed N` | 不指定则随机；整数 0–4294967295 | 支持；V8.X 官方称约 99% 一致，不代表跨版本/参数/时点必然逐像素一致，也不能用来保存人物/风格 | [种子]、[版本] |
| 排除元素 | `--no item1, item2` | 可选文本列表，没有数字默认值 | 支持；每个词独立解释，不能把复杂“不要某种关系”当可靠语义编辑 | [排除]、[版本] |
| 减少自动美化 | `--raw` | 可选开关；默认可能被账户设置改变 | 支持；减少模型默认审美，让提示词更直接影响结果；不等于所有图都会更写实 | [Raw]、[版本] |
| 无缝贴图 | `--tile` | 可选开关 | 支持，2026-09-24 更新改善 V8.1/8.2 拼接；仅生成一个 tile；上采样可能破坏无缝性；与 Edit Model 不兼容 | [平铺]、[编辑模型]、[9月更新] |
| 普通参考图强度 | `--iw N` | V8.1 默认 1；0–3 | 支持；影响普通 Image Prompt 对结果的作用，不是风格图权重，也不是逐张图片独立权重 | [图像提示]、[版本] |
| 风格参考 | `--sref URL1 URL2` / `--sref CODE` / `--sref random` | 可选；URL、数字风格码、random；官方未在本文来源给普通 sref 数量上限 | 支持；参考颜色、材质、光线、媒介等风格，不保留人物物体身份；需要文字正文 | [风格参考]、[版本] |
| 风格参考强度 | `--sw N` | 默认 100；0–1000 | 支持；作用于 sref；不能用于 Moodboard | [风格参考]、[版本] |
| 风格参考算法版 | `--sv N` | V7 为 1–6、默认 6；V6 为 1–4、默认 4 | **V8.1 的独立枚举和默认未确证**；不能仅因 V8.1 支持 sref 就把 V7 的 sv 枚举强塞给它 | [风格参考] |
| 标准/高清 | `--sd` / `--hd` | 标准 1024px / 高清 2048px；1:1 时分别约 1024²/2048² | 支持；V8.1 发布公告曾把 HD 设为默认，但当前账户设置和 Magic 未指定时的行为不能由历史公告推断；产品应显式选一项 | [参数总览]、[版本]、[V8.1公告]、[图像尺寸] |
| 速度 | `--fast` / `--relax` | 由账户设置决定默认；参数可覆盖 | 支持；Relax 排队在官方服务可达 0–30 分钟；官方订阅资格是账户限制，不代表第三方计费 | [GPU速度]、[版本] |
| 个性化 | `--p` / `--profile`，或 `--p PROFILE_ID/CODE` | 单独 `--p` 使用当前账户默认 profile；指定 ID/code 使用对应配置 | 支持，但依赖上游账户；需要解锁 Global Profile；不能把池化渠道账户 profile 当下游用户自己的偏好 | [个性化]、[版本]、[参数总览] |
| V8 指令编辑/主体参考 | `--edit URL1 URL2` | 最多 4 张；未提供通用强度参数 | 官方支持 V8.1/8.2；默认沿用第一张图比例，显式 `--ar` 可覆盖；这是独立参考角色，不能假定 Magic 的普通 base64Array 自动等价 | [编辑模型] |

数值表按官方原值记录；除 seed 和画幅分量已明确整数外，上述资料未统一规定各滑杆必须整数或统一步长。后续产品若用整数滑杆，应把步长标成产品约束，而非冒称官方限制。[画幅]、[种子]

### 分辨率、比例、成本要分开

- `--ar` 控制形状；`--sd/--hd` 控制生成分辨率等级；它们不能承诺任意 `width × height` 精确像素。[画幅]、[图像尺寸]
- 当前官方文档给普通 SD prompt 约 0.8 GPU 分钟，HD 约 1.3；Edit SD 约 1，Edit HD 约 2.3。它们是官方 GPU 计量，不是 Magic 售价，不是实际等待秒数。[GPU速度]
- 当前官方 GPU 页没有把“HD 静态图片”列为 Relax 禁用；Relax 禁用的是 HD **视频**、repeat、permutations、legacy Max Upscale。不能把视频限制误扩展到静态 HD 图片；Magic 的 `--hd --relax` 仍需渠道验证。[GPU速度]
- V8.1 发布公告称 HD 默认；这是 2026-04-14 的产品发布行为。当前官方设置允许切换 SD/HD，本文不保证上游池化账户省略参数时的默认分辨率。[V8.1公告]、[版本]

## 不应按 V8.1 可用项直接展示的参数

| 参数 | 正确所属 | V8.1 判断 | 开发提醒 / 来源 |
| --- | --- | --- | --- |
| `--q` / `--quality` | V7：1（默认）/2/4；V6：0.5/1/2 | 不支持 | 改变计算投入，不是固定像素尺寸；V7 的 q3 会变 q4；q4 与 Omni Reference 不兼容。[质量]、[版本] |
| `--stop` | V6 及以前，10–100，默认 100 | 不支持 | 旧教程的“中途停止”不能套到 V8.1。[遗留功能] |
| `--cref URL` / `--cw` | MJ6/Niji6 的 Character Reference；cw 默认和最大 100 | 不支持 | 当前 Legacy 文档未明确写 cw 最小值，本次不凭记忆补范围；V7 已换 Omni，V8 已换 Edit。[遗留功能] |
| `--oref URL` / `--ow N` | V7 Omni Reference，单图；ow 1–1000、默认 100 | 不支持 | 官方建议通常低于 400；不能为 V8.1 UI 提供“Omni 权重”然后指向 iw。[Omni]、[版本] |
| `--turbo` | 部分较早模型的 GPU 速度 | 不支持 V8.1 | GPU 文档泛称 V5+ 可 Turbo，但本页兼容提示和版本矩阵明确排除 V8.1；不要只读泛化段落。[GPU速度]、[版本] |
| `--niji 7` | 独立 Niji 动漫模型，2026-01-09发布；Niji6 是旧版 | 不是 V8.1 的风格开关 | 不应与 `--v 8.1` 并列当两个可自由组合参数；如以后接入，按不同模型能力配置。[版本] |
| `--draft` | V7 可用；V8.1/V8.2 **仅官网** | 官方网页支持；Discord 不支持 | V8 生成 24 张 512²，0.4 GPU 分钟，与当前四候选 Imagine 合同不同；Magic 未确证。[草稿]、[随机草稿]、[10月更新] |
| `--repeat N` / `--r N` | 批量重复请求；Basic 2–4，Standard 2–10，Pro/Mega 2–40 | 版本本身有能力，但 Relax 不支持 | Discord 还要点确认；会增加生成任务与成本，不等价于当前同步端点 `n`。[重复]、[GPU速度] |
| `--stealth` / `--public` | 官方账号公开性模式，Stealth 限 Pro/Mega | 不应当作下游可靠隐私开关 | Discord 公共频道即使 Stealth 也仍可见；第三方代理 URL、日志和存储不由该参数保护。[隐身] |
| `--motion low/high`、`--loop`、`--end`、`--bs`、`--video` | 视频工作流 | 不属本次静态 Imagine 产品 | 参数总览中存在不意味着图片端点可用。[参数总览] |

### `::` 有两个容易混淆的用途

1. **文本多段加权**：`space::2 ship::1`；官方明确仅支持到 MJ6.1（含列出的旧版/Niji），当前版本表 V7/V8.1/V8.2 均不支持。不要为 V8.1 做“关键词权重”并输出这类字符串。[多提示]、[版本]
2. **Style Reference 之间的相对权重**：风格页仍给 Discord 语法 `--sref URL1::2 URL2::1 URL3::1`；这是 sref 内部权重，并不等同于文本 Multi-Prompt。此段没有单独承诺 V8.1 的具体版本行为，且当前渠道对带权重 URL 的处理未验证，宜列高级待验项。[风格参考]

对普通多图 Image Prompt，官方当前页面只明确多 URL 和整体 `--iw`，没有给出 V8.1 每张图单独 `URL::weight` 的支持合同。不能从 sref 加权示例推导普通参考图可以逐张加权。[图像提示]、[风格参考]

## 图片角色与界面语义

| 用户意图 | 官方机制 | 示例表达 | 应该解释给用户的作用 | 来源 |
| --- | --- | --- | --- | --- |
| 根据图重新创作 | 普通 Image Prompt | Discord 把 URL 放正文最前；多图用空格隔开，参数 `--iw 1.5` | 借鉴内容、构图、颜色，不是精准修图 | [图像提示] |
| 使用相同画风 | Style Reference | `--sref https://... --sw 100` | 借鉴风格，不是保留同一个人 | [风格参考] |
| 保持人物/物体、按指令改图 | V8 Edit Model | `--edit https://...`，最多四图 | 可描述“把围巾换成红色”等指令，不要求只写最终场景 | [编辑模型] |
| 同一角色旧版工作流 | V7 Omni / V6 Character | `--oref` / `--cref` | 需要切换兼容版本，不能在 V8.1 原样使用 | [Omni]、[遗留功能] |
| 自己训练/挑选的审美 | Personalization / Moodboards | `--p ...` | 是账户/资料相关配置，不是仅上传一张普通参考图 | [个性化] |

普通参考图：一张图必须加文字，或两张及以上可不加文字；纯多图无文字的普通 Image Prompt 不兼容 `--stylize`/`--weird`。官方建议输入图裁成目标比例以改善结果。这是官方使用方式，当前下游 API 是否允许空 prompt 仍由本项目接口合同决定。[图像提示]

普通参考图、Style Reference 当前官方均列出 png/gif/webp/jpg/jpeg；本项目只支持哪些输入/输出预处理格式，应以项目实现为准，不能把官方 GIF 支持扩展成代理已实现。[图像提示]、[风格参考]

## 官方最新与文档冲突记录

| 问题 | 看到的一手证据 | 调研结论 |
| --- | --- | --- |
| Draft 是否支持 V8 | Parameter List 仍称 V7；Version 表 V8 列打叉；Draft 详细页明确 V8.1/V8.2 网页 24 张，2026-06-25 公告和 10-08 changelog 再次确认 | 以细分渠道说明和更新公告优先：V8 官网支持，Discord 不支持；不能输出“V8 所有渠道不支持 Draft”。[参数总览]、[版本]、[草稿]、[随机草稿]、[10月更新] |
| Omni 的 Fast Mode | Omni 详细页写“not compatible with Fast Mode”；旧公告提到 fast-mode 是独立实验功能，容易与 GPU Fast 混淆 | 本次目标 V8.1 明确不支持 Omni，因此不为 V7 的不同 Fast 语义补推断；若未来接 V7 Omni，再核实。[Omni]、[实验参数] |
| Legacy 页“当前默认 V7”、废弃 `--fast` | 同页保留历史内容；当前 Version 页写默认 V8.2，GPU页写当前 `--fast` | 不应读成现在的 GPU `--fast` 已废弃；参数名可在历史上被复用。[遗留功能]、[版本]、[GPU速度] |
| V8.1 sv 数值 | Style Reference 只逐项写 V6/V7，不单列 V8.1 | sref/sw 可确认，V8.1 sv 默认/枚举列未知；不从V7静默继承。[风格参考]、[版本] |
| V8.1 高清默认 | 4月公告写 HD 成为默认；当前可以改网页设置，第三方渠道另有账户/版本 | 记录历史默认，但产品显式发 sd/hd 并测渠道。[V8.1公告]、[版本] |
| Thinking Mode | 2026-10-08 公告为 alpha 网页下的 Rerun (Thinking)；changelog 明确 V8.2 standard/edit | 没有核实到公开 `--thinking` 语法；不把它捏造成提示词参数。[Thinking]、[10月更新] |

## 对后续产品的研究建议（尚未实施）

首屏可用“画幅 + 速度 + 标准/高清 + Raw + 参考图”；高级区可用“风格化、多样性、怪诞、seed、排除元素”，有普通参考图时再显示 `iw`；风格参考应独立于普通参考图设置 `sref/sw`。这是基于上表能力的产品建议，不是官方 UI 必须遵循的结构。

版本应作为能力配置的键：当前项目锁 V8.1，则不展示 q/stop/cref/oref/turbo；Niji 作为另一模型；Draft、Edit、逐张权重、sv 等保持待渠道验收。请求正文与参数分别维护，最终统一序列化为正文 + 参数后缀，可以避免用户手写参数与控件互相覆盖。

本资料只承担官方事实。Magic 和当前代理的证据、透传边界与验收优先级见[参数产品化综合调研](2026-10-22-midjourney-product-controls.md)；普通 `image` 参考图不能仅凭官方文档当成 sref/Edit 图片。

## 来源索引与获取方式

以下来源均于 2026-10-22 访问。官方文档 HTML 部分触发 Cloudflare，本次使用同一官方站点公开 Zendesk JSON 读取完整 `article.body`，不是二手摘要：`https://docs.midjourney.com/api/v2/help_center/en-us/articles/{id}.json`。更新时间来自 JSON `updated_at`；公告日期来自页面 JSON-LD。表内引用链接指向可阅读的官方文章。

| 来源 | 官方文章更新 / 发布日期 |
| --- | --- |
| [参数总览] | 2026-09-01 |
| [版本] | 2026-09-01 |
| [画幅] | 2026-07-27 |
| [风格化] | 2026-07-27 |
| [多样性] | 2026-07-27 |
| [怪诞] | 2026-07-27 |
| [质量] | 2026-07-27 |
| [种子] | 2026-07-27 |
| [排除] | 2026-07-27 |
| [Raw] | 2026-07-27 |
| [平铺] | 2026-07-27 |
| [图像提示] | 2026-08-31 |
| [风格参考] | 2026-09-01 |
| [Omni] | 2026-09-01 |
| [编辑模型] | 2026-09-24 |
| [遗留功能] | 2026-09-01 |
| [GPU速度] | 2026-08-28 |
| [草稿] | 2026-07-27 |
| [个性化] | 2026-10-15 |
| [重复] | 2026-07-27 |
| [多提示] | 2026-07-27 |
| [隐身] | 2026-03-25 |
| [图像尺寸] | 2026-07-27 |
| [实验参数] | 2025-04-30 |
| [V8.1公告] | 2026-04-14 |
| [V8.2公告] | 2026-07-24 |
| [随机草稿] | 2026-06-25 |
| [9月更新] | 2026-09-24 |
| [10月更新] | 2026-10-08（文章最后修改 10-09） |
| [Thinking] | 2026-10-08 |

[参数总览]: https://docs.midjourney.com/hc/en-us/articles/32859204029709-Parameter-List
[版本]: https://docs.midjourney.com/hc/en-us/articles/32199405667853-Version
[画幅]: https://docs.midjourney.com/hc/en-us/articles/31894244298125-Aspect-Ratio
[风格化]: https://docs.midjourney.com/hc/en-us/articles/32196176868109-Stylize
[多样性]: https://docs.midjourney.com/hc/en-us/articles/32099348346765-Chaos-Variety
[怪诞]: https://docs.midjourney.com/hc/en-us/articles/32390120435085-Weird
[质量]: https://docs.midjourney.com/hc/en-us/articles/32176522101773-Quality
[种子]: https://docs.midjourney.com/hc/en-us/articles/32604356340877-Seeds
[排除]: https://docs.midjourney.com/hc/en-us/articles/32173351982093-No
[Raw]: https://docs.midjourney.com/hc/en-us/articles/32634113811853-Raw
[平铺]: https://docs.midjourney.com/hc/en-us/articles/32197978340109-Tile
[图像提示]: https://docs.midjourney.com/hc/en-us/articles/32040250122381-Image-Prompts
[风格参考]: https://docs.midjourney.com/hc/en-us/articles/32180011136653-Style-Reference
[Omni]: https://docs.midjourney.com/hc/en-us/articles/36285124473997-Omni-Reference
[编辑模型]: https://docs.midjourney.com/hc/en-us/articles/48495453462797-Edit-Model
[遗留功能]: https://docs.midjourney.com/hc/en-us/articles/33329788681101-Legacy-Features
[GPU速度]: https://docs.midjourney.com/hc/en-us/articles/32016412137741-GPU-Speed-Fast-Relax-Turbo
[草稿]: https://docs.midjourney.com/hc/en-us/articles/35577175650957-Draft-Conversational-Modes
[个性化]: https://docs.midjourney.com/hc/en-us/articles/32433330574221-Personalization
[重复]: https://docs.midjourney.com/hc/en-us/articles/32757107922061-Repeat
[多提示]: https://docs.midjourney.com/hc/en-us/articles/32658968492557-Multi-Prompts-Weights
[隐身]: https://docs.midjourney.com/hc/en-us/articles/32019750070669-Stealth-Mode
[图像尺寸]: https://docs.midjourney.com/hc/en-us/articles/33329374594957-Image-Size-Resolution
[实验参数]: https://updates.midjourney.com/v7-update-editor-and-exp/
[V8.1公告]: https://updates.midjourney.com/v8-1-alpha/
[V8.2公告]: https://updates.midjourney.com/version-8-2/
[随机草稿]: https://updates.midjourney.com/random-styles-in-draft-mode/
[9月更新]: https://updates.midjourney.com/edit-updates-thumbnail-previews-and-more/
[10月更新]: https://updates.midjourney.com/alpha-changelog-10-7-26/
[Thinking]: https://updates.midjourney.com/testing-thinking-mode/
