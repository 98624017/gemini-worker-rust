# Niji 7 官方参数调研与 V8.2 差异

调研日期：2026-10-22（Asia/Shanghai）。目的：支持Midjourney V8.2与Niji 7双模型产品规划。参数功能、范围和默认值以官方说明为依据；版本专属定义优先，其余采用官方通用参数定义。

## 版本结论

本次可核实的最新 Niji 是 **Niji 7**。Midjourney 当前 Version 文档列 Niji 7 于 2026-01-09 发布；Niji 官方博客当前索引最高版本标签是 Niji v7，最新相关功能文章仍围绕 Niji 7。未找到更新 Niji 主版本的官方发布依据，不能凭 MJ 已到 V8.2 推断存在 Niji 8。[MJ版本]、[Niji博客]、[Niji7发布]、[切换模型]

Niji 官方明确选择语法为 `--niji 7`，网页 Version 下拉框可选 Niji 7。主模型 V8.2 使用 `--v 8.2`，它们是两个模型选择，产品应二选一，不应生成同时包含两者的请求。官方没有在本次来源里给出两者同时出现时的优先级，不能依赖“最后一个生效”。[Niji7发布]、[MJ版本]

Niji 7 比旧版更按文字逐项作画，默认画面更扁平，线条更清晰；背景、灯光、景别等可在正文明确描述。官方提示可用 `indoors`/`outdoors` 补背景、用鞋子描述帮助生成全身，不必为了表现这些内容发明新的 `--` 参数。[Niji7发布]、[Niji7提示指南]

## Niji 7 参数说明

Niji专属范围直接使用版本表；风格化、chaos、seed和风格权重采用官方通用参数定义。官方未说明的项目不列入控件规划。Midjourney Version表的V7列指MJ V7，不能直接替代Niji7的参数列表。[MJ版本]

| 功能 / 参数 | 官方功能说明 | 默认／范围 | 控件建议 | 来源 |
| --- | --- | --- | --- | --- |
| 模型 `--niji 7` | **确认** | 显式选择 Niji 7；不依赖账户默认 | 与 `--v 8.2` 二选一 | [Niji7发布]、[切换模型] |
| 画幅 `--ar` / `--aspect` | 调整宽高比 | 默认1:1，正整数比例 | 常用比例按钮 | [Niji画幅]、[MJ画幅] |
| 风格化 `--s` / `--stylize` | 调整模型美学介入程度 | 0–1000，默认100；官方通用定义 | 滑块＋数值输入 | [MJ风格化]、[Niji权重] |
| 多样性 `--c` / `--chaos` | 增加候选间差异 | 0–100，默认0；官方通用定义 | 变化度滑块 | [Niji7提示指南]、[MJ多样性] |
| `--raw` | 减弱默认模型风格；Niji7可减少默认动漫倾向 | 开关；省略时沿用账号设置 | 自动／开启 | [Niji7提示指南]、[MJRaw] |
| 排除 `--no` | Niji 帮助中心确认 | 支持多个排除元素，逗号分隔；不是数值滑杆 | 标签输入；实际 ASCII 参数用 `--no`，不要复制帮助页排版中的长破折号 | [Niji排除]、[MJ排除]、[MJ参数总览] |
| 随机种子 `--seed` | 控制生成初始随机噪声 | 整数0–4294967295，默认随机；官方通用定义 | 随机／指定数值 | [MJ种子] |
| 普通参考图 `--iw` | **确认** | **Niji 7 默认 1，范围 0–2**；Image Prompts 页有专列 | 与主模型分别约束；不能直接套 V8.1 的 0–3 | [MJ图像提示]、[Niji普通参考] |
| 风格参考 `--sref` | **确认** | Niji 7 发布文明确 sref，提示指南明确用旧版结果图作为新模型风格图；最新 Style Creator 支持 Niji 7 风格代码 | 普通参考图与风格图区分；图片和代码都应保存所属模型 | [Niji7发布]、[Niji7提示指南]、[Niji风格参考]、[Niji风格创建] |
| 风格权重 `--sw` | 控制风格参考影响 | 0–1000，默认100；官方通用定义 | 有风格参考时显示 | [Niji权重]、[MJ风格参考] |
| 风格算法版 `--sv` | 官方明确支持sv4兼容 | `--sv 4` | 可提供旧风格参考兼容选项，默认不额外附加sv | [Niji7发布]、[MJ风格参考] |
| 指令编辑 `--edit` | **明确不在当前官方兼容范围** | Edit Model 官方限定 MJ8.1/8.2，未列 Niji 7 | 不把 V8 Edit 一致性功能自动移植到动漫模型 | [MJ编辑模型] |
| 角色参考 `--cref` / `--cw` | **明确 Niji 7 不支持 cref** | 发布文说 cref 不会支持；Legacy将其限 MJ/Niji6 | 不提供 Niji 7角色锁定滑杆；普通iw不是cw替代 | [Niji7发布]、[MJ遗留功能] |
| Omni `--oref` / `--ow` | **明确不在当前官方兼容范围** | 官方 Omni限定 Midjourney V7；不是 Niji7 | Niji通用权重指南出现ow不构成Niji7兼容证明 | [MJOmni]、[Niji权重] |
| 个性化 `--p` / `--profile` | 当前已有Global Niji V7 Profile | 使用默认profile或指定ID/code | 账号个性化功能独立规划 | [Niji个性化]、[MJ个性化] |
| `--fast` / `--relax` | 生成速度模式 | Fast默认；Relax需官方Standard+订阅 | 当前产品提供FAST／RELAX | [Niji速度]、[MJGPU速度] |
| `--style cute/scenic/expressive/original` | **不可继承为 Niji 7合同** | 明确是 Niji 5 的旧风格；Niji7指南建议先旧模型生成再sref | 不应称其为Niji6或Niji7自带档位；可作为普通正文风格词，但效果另论 | [MJ遗留功能]、[Niji7提示指南] |

参数列表只收录官方有依据的项目。产品滑块的步长和常用比例按钮属于界面设计，不改变官方参数数值范围。

## 与 V8.2 两条产品线的关键差异

| 维度 | Midjourney V8.2 | Niji 7 | 产品结论 |
| --- | --- | --- | --- |
| 选择 | `--v 8.2`，当前MJ默认 | `--niji 7`，最新可核实Niji | 分开选择一个，不同时输出 |
| 原生分辨率档位 | SD/HD，比例极限14:1/4:1 | 不列入该模型选项 | V8.2显示分辨率档位 |
| 普通图片强度 | 支持iw | 0–2、默认1 | 按模型分别配置，不人为统一上限 |
| 主体一致性/编辑 | Edit最多4图 | cref不支持；Edit属于V8功能 | 单独设计V8编辑入口 |
| 风格参考 | sref/sw | sref/sw、sv4兼容 | 风格参考与普通参考图分区 |
| Raw语义 | 减少MJ自动审美 | 官方指南特指可“less anime” | 帮助文案按模型调整 |
| 旧风格档位 | 不适用Niji5旧style | 同样不能直接继承 | 使用正文描述或单独style reference |

上表依据：[MJ版本]、[MJ图像提示]、[MJ编辑模型]、[MJ风格参考]、[Niji7发布]、[Niji7提示指南]。

## 参考图与界面提醒

Niji官方把 Style Reference 和 Image Prompt 分开教，权重指南把 iw/sw/cw/ow列为不同类别。该指南同时覆盖旧模型，不意味着所有类别在Niji7都可用。尤其“Using character references”帮助页是通用站点内容，不能推翻Niji7专门发布文中“cref不会支持”的明确判断。[Niji风格参考]、[Niji普通参考]、[Niji权重]、[Niji角色帮助]、[Niji7发布]

`--no` 的多词短语并非可靠整体否定；MJ官方例子 `--no modern clothing` 会把modern和clothing分别处理。输入排除词建议用具体元素；更复杂的服装、场景关系优先在正文正向描述。[Niji排除]、[MJ排除]

前端使用共同基础布局，按模型显示特有控件。Niji7提供正文、普通参考图及iw、比例、风格化、chaos、Raw、No、seed和sref/sw，风格参考可附加sv4兼容；数值按上表官方定义校验。

## 一手来源与读取方式

以下全部于2026-10-22访问。MJ文档通过官方公开Zendesk JSON读取完整article.body；Niji官网直连Cloudflare时，使用Jina Reader作为传输读取 `URL Source: https://nijijourney.com/...` 的原文。内容权威来源仍是链接指向的Niji官网，不采用Reader生成的搜索回答、二手文章或推测。Niji页大多没有返回可核验更新时间，不能把访问日期当发布日期。

最新版本依据官方Version文档、Niji博客索引与模型选择帮助页。参数说明优先采用Niji7专属定义，其余引用官方适用的通用说明。

[MJ版本]: https://docs.midjourney.com/hc/en-us/articles/32199405667853-Version
[MJ参数总览]: https://docs.midjourney.com/hc/en-us/articles/32859204029709-Parameter-List
[MJ画幅]: https://docs.midjourney.com/hc/en-us/articles/31894244298125-Aspect-Ratio
[MJ风格化]: https://docs.midjourney.com/hc/en-us/articles/32196176868109-Stylize
[MJ多样性]: https://docs.midjourney.com/hc/en-us/articles/32099348346765-Chaos-Variety
[MJRaw]: https://docs.midjourney.com/hc/en-us/articles/32634113811853-Raw
[MJ排除]: https://docs.midjourney.com/hc/en-us/articles/32173351982093-No
[MJ种子]: https://docs.midjourney.com/hc/en-us/articles/32604356340877-Seeds
[MJ图像提示]: https://docs.midjourney.com/hc/en-us/articles/32040250122381-Image-Prompts
[MJ风格参考]: https://docs.midjourney.com/hc/en-us/articles/32180011136653-Style-Reference
[MJ怪诞]: https://docs.midjourney.com/hc/en-us/articles/32390120435085-Weird
[MJ实验参数]: https://updates.midjourney.com/v7-update-editor-and-exp/
[MJ质量]: https://docs.midjourney.com/hc/en-us/articles/32176522101773-Quality
[MJ平铺]: https://docs.midjourney.com/hc/en-us/articles/32197978340109-Tile
[MJ9月更新]: https://updates.midjourney.com/edit-updates-thumbnail-previews-and-more/
[MJ编辑模型]: https://docs.midjourney.com/hc/en-us/articles/48495453462797-Edit-Model
[MJ遗留功能]: https://docs.midjourney.com/hc/en-us/articles/33329788681101-Legacy-Features
[MJOmni]: https://docs.midjourney.com/hc/en-us/articles/36285124473997-Omni-Reference
[MJ个性化]: https://docs.midjourney.com/hc/en-us/articles/32433330574221-Personalization
[MJGPU速度]: https://docs.midjourney.com/hc/en-us/articles/32016412137741-GPU-Speed-Fast-Relax-Turbo
[Niji博客]: https://nijijourney.com/blog
[Niji7发布]: https://nijijourney.com/blog/niji-7
[Niji7提示指南]: https://nijijourney.com/blog/niji-7-prompting
[Niji风格创建]: https://nijijourney.com/blog/style-creator
[切换模型]: https://nijijourney.com/help-center/app-new-user-switching-model
[Niji画幅]: https://nijijourney.com/help-center/app-new-user-aspect-ratio
[Niji速度]: https://nijijourney.com/help-center/app-new-user-what-is-fast-turbo-and-relaxed
[Niji个性化]: https://nijijourney.com/help-center/app-new-user-personalization
[Niji风格参考]: https://nijijourney.com/help-center/app-advanced-style-ref
[Niji普通参考]: https://nijijourney.com/help-center/app-advanced-image-ref
[Niji权重]: https://nijijourney.com/help-center/app-advanced-weighted-references
[Niji排除]: https://nijijourney.com/help-center/app-advanced-negative-prompting
[Niji角色帮助]: https://nijijourney.com/help-center/app-advanced-character-ref
