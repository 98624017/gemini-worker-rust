# Midjourney 参数前端开源项目参考

调研日期：2026-10-22。目标是为 Midjourney V8.2 与 Niji V7 的参数面板选参考，核对 GitHub 仓库元数据、README 和实际源码，不安装或运行候选项目。星标和推送时间只是检索时快照，不等于功能验收或发布质量。

## 推荐结论

优先参考 [Dooy/chatgpt-web-midjourney-proxy](https://github.com/Dooy/chatgpt-web-midjourney-proxy) 的生图交互，再参考 [ChatAnyTeam/ChatAny](https://github.com/ChatAnyTeam/ChatAny) 的参数表单。两者都有实际前端实现和较大社区；本次未找到已经完整配置并验证 Midjourney V8.2 与 Niji V7 两套参数的成熟开源前端。

| 项目 | 检索时状态 | 适合借鉴 | 目标版本边界 |
| --- | --- | --- | --- |
| Dooy/chatgpt-web-midjourney-proxy | 约 6.8k stars；MIT；最近推送 2026-09-29 | 比例按钮、风格化档位、版本选择、不同角色参考图输入，以及生成任务与结果交互 | 版本配置已包含 `--niji 7`，MJ 最高为 `--v 7.0`；未发现 V8.2 配置。配置存在不代表本次实测过 Niji 7 渠道。 |
| ChatAnyTeam/ChatAny | 约 6.5k stars；MIT；最近推送 2026-04-02 | 配置驱动的参数表单、图片权重数值框、比例、seed、排除元素、MJ/Niji 模式切换 | 参数配置偏旧；例如图片权重为 0–2、默认 2，不能直接作为 V8.2/Niji V7 的范围依据。 |

第一项直接证据：

- [参数控件 aiDrawInputItem.vue](https://github.com/Dooy/chatgpt-web-midjourney-proxy/blob/634f8f51e77d677ee56296b02d9ee8e02065886a/src/views/mj/aiDrawInputItem.vue)。
- [版本及选项配置 draw.json](https://github.com/Dooy/chatgpt-web-midjourney-proxy/blob/634f8f51e77d677ee56296b02d9ee8e02065886a/src/views/mj/draw.json)。风格化预设为 50 / 100 / 250 / 750；不是完整数值滑杆，亦未据此认定其包含独立 iw / chaos 控件。
- [仓库元数据](https://api.github.com/repos/Dooy/chatgpt-web-midjourney-proxy)。

第二项具体源码证据与进一步候选见[候选核查记录](2026-10-22-midjourney-ui-projects-candidates.md)。

## 次要参考：提示词预设工具

[Amery2010/midjourney-prompt-generator](https://github.com/Amery2010/midjourney-prompt-generator)：约 188 stars，MIT，最近推送 2025-12-06。使用 Next.js、TypeScript、Tailwind，适合参考轻量预设选择和中文交互，不属于完整 MJ 生图前端。

其 [preset-list.tsx](https://github.com/Amery2010/midjourney-prompt-generator/blob/ff69f7f18b35d7660079d3cb48ac6dc17f0b8d45/src/components/preset-list.tsx) 有比例、绘画风格、色彩、光照、构图下拉框；[页面](https://github.com/Amery2010/midjourney-prompt-generator/blob/ff69f7f18b35d7660079d3cb48ac6dc17f0b8d45/src/app/%5Blocale%5D/page.tsx) 将 `{ model, presets, text }` 发送到 `/api/prompt` 由语言模型生成提示词，结果用于复制。没有证据表明它提供直接 MJ 生图、完整 `--s` / `--iw` 调参或 V8.2/Niji V7 兼容配置。这里的“图片转提示词”也不是图片作为 MJ 参考图直接生成。

来源：[中文 README](https://github.com/Amery2010/midjourney-prompt-generator/blob/ff69f7f18b35d7660079d3cb48ac6dc17f0b8d45/README.zh-CN.md)、[仓库元数据](https://api.github.com/repos/Amery2010/midjourney-prompt-generator)。

## 对本项目的建议

仅借鉴参数区及输入组织，无需引入整站聊天、账号或另一套 MJ 后端。现有同步 OpenAI 图片接口继续负责生成；前端维护用户正文和选项，由确定性序列化生成最终 prompt。绘画风格预设是正文描述，`--s` 是模型风格化程度，两者不要混为一个选项。

建立两套能力配置：V8.2 输出 `--v 8.2`，Niji V7 输出 `--niji 7`；不要从旧项目复制其全部参数和范围，也不要同时拼接两种版本选择。Niji V7 的 Magic 渠道可用性仍需单独实测。V8.2 已验证范围见[本项目实测记录](../validation/2026-10-22-magic-midjourney-v82.md)，官方参数基线见[参数调研](2026-10-22-midjourney-official-parameters.md)。

产品首层可放模型、比例、参考图和速度；高级区放风格化、参考图权重、变化度、Raw、seed、排除元素。每个版本独立控制可见性、合法范围和互斥规则。该布局为产品建议，尚未实施。
