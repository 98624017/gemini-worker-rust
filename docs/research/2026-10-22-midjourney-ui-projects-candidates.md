# Midjourney 参数控件整站候选调研

调研日期：2026-10-22。目标为 Midjourney V8.2 与 Niji V7；仅查一手 GitHub README、源码、许可证和提交记录，没有部署或验证生成结果。

## 结论

本轮找到两个有实际参数控件的成熟整站，但没有证据表明其中任何一个同时完整支持 V8.2 与 Niji V7。优先借鉴 Dooy 的现成绘图操作面板；需要 `iw` 控件、声明式字段配置和参数拼接时参考 ChatAny。两者使用 Midjourney Proxy 风格的异步接口，接本仓库 OpenAI 同步入口仍需替换请求层，不能直接认为开箱即用。

## 1. Dooy/chatgpt-web-midjourney-proxy：最接近目标

- [仓库](https://github.com/Dooy/chatgpt-web-midjourney-proxy)，GitHub API 快照为 6,802 stars；[MIT 许可证](https://github.com/Dooy/chatgpt-web-midjourney-proxy/blob/634f8f51e77d677ee56296b02d9ee8e02065886a/license)。[仓库元数据](https://api.github.com/repos/Dooy/chatgpt-web-midjourney-proxy)的最近 push 为 2026-09-29；[HEAD 提交](https://github.com/Dooy/chatgpt-web-midjourney-proxy/commit/634f8f51e77d677ee56296b02d9ee8e02065886a)为同日。
- [README_ZH.md](https://github.com/Dooy/chatgpt-web-midjourney-proxy/blob/634f8f51e77d677ee56296b02d9ee8e02065886a/README_ZH.md)有文生图、垫图、MJ/Niji、局部重绘、变体等功能及界面截图；说明使用 `midjourney-proxy` / `midjourney-proxy-plus` 后端。
- **真实控件**：[src/views/mj/aiDrawInputItem.vue](https://github.com/Dooy/chatgpt-web-midjourney-proxy/blob/634f8f51e77d677ee56296b02d9ee8e02065886a/src/views/mj/aiDrawInputItem.vue) 中第 364 行起为比例图形按钮，第 378 行起用 Naive UI `n-select` 渲染风格、风格化、版本等选项；`createPrompt` 在第 140–161 行将控件值合并成 `--s`、`--ar`、版本及参考图参数。第 383–413 行还有 `cw` 数值框及 `sref`、`cref`、`oref` 输入/上传控件。
- **风格化和版本证据**：[src/views/mj/draw.json](https://github.com/Dooy/chatgpt-web-midjourney-proxy/blob/634f8f51e77d677ee56296b02d9ee8e02065886a/src/views/mj/draw.json) 将风格化分为 `--s 50/100/250/750` 四档；版本列表明确包含 `--niji 7`，MJ 最高选项仍为 `--v 7.0`，未见 V8.2。不要将“可在提示词手写版本”当成已有 V8.2 控件或兼容校验。
- **边界**：上述主面板未发现 `iw` / `chaos` 专门字段；已有 `quality` 会拼 `--q`，且参考类型同时展现，不能直接照搬到两个新版。应按官方版本能力矩阵重新筛选控件。
- **相关文件活跃性**：[draw.json 最近提交](https://github.com/Dooy/chatgpt-web-midjourney-proxy/commit/cb660eaffa85115edd5583c113cd68ac9eb2e1c9)为 2026-03-05；[aiDrawInputItem.vue 最近提交](https://github.com/Dooy/chatgpt-web-midjourney-proxy/commit/a45a5cce2adcf89d802aa12016f630bdc2c79d72)为 2025-08-19。仓库整体活跃不等于该参数面板持续跟进最新模型。

## 2. ChatAnyTeam/ChatAny（原 Licoy/ChatGPT-Midjourney）：`iw` 和 Schema 面板参考

- [仓库](https://github.com/ChatAnyTeam/ChatAny)，GitHub API 快照为 6,485 stars；[MIT 许可证](https://github.com/ChatAnyTeam/ChatAny/blob/0210f56af03220b75cea1375f5f920df13f93752/LICENSE)。[仓库元数据](https://api.github.com/repos/ChatAnyTeam/ChatAny)和 [HEAD 提交](https://github.com/ChatAnyTeam/ChatAny/commit/0210f56af03220b75cea1375f5f920df13f93752)显示最近 push / 提交为 2026-04-02。
- [README.md](https://github.com/ChatAnyTeam/ChatAny/blob/0210f56af03220b75cea1375f5f920df13f93752/README.md)有 MJ 生成主界面和区域重绘截图；明确使用第三方 `midjourney-proxy` 类 API。
- **真实控件**：[app/components/mj/mj-panel.tsx](https://github.com/ChatAnyTeam/ChatAny/blob/0210f56af03220b75cea1375f5f920df13f93752/app/components/mj/mj-panel.tsx) 的 `imagineParams` 定义 MJ/Niji 切换、版本下拉、负面提示词、参考图片上传、`iw` 数值框、比例和 seed；`ControlParam` 按 `textarea/select/number/images/models` 渲染真正的 React 表单。
- **参数拼接证据**：[app/components/mj/mj-sidebar.tsx](https://github.com/ChatAnyTeam/ChatAny/blob/0210f56af03220b75cea1375f5f920df13f93752/app/components/mj/mj-sidebar.tsx) 第 45–95 行读取字段配置，将 `cmd` 与值追加到用户提示词，并把图片转为 `base64Array`。可借鉴“配置定义控件 + 统一序列化”结构。
- **明显过时的模型和范围**：源码仅列 V6 / V5.2 / V5.1 / V5；`iw` 定义为默认 2、范围 0–2、最多两张参考图。未见 V8.2 / Niji V7 选项。Niji 选择路径是追加裸 `--niji`，不是明确 `--niji 7`。这些旧值不应成为新产品规范。
- **相关文件活跃性**：[mj-panel.tsx 最近提交](https://github.com/ChatAnyTeam/ChatAny/commit/7e6ba612e941805f15e8147e3391823bddb71482)为 2024-08-06；主参数面板明显早于目标版本。

## 未选为参数控件参考：ConnectAI-E/MidJourney-Web

[仓库](https://github.com/ConnectAI-E/MidJourney-Web)和 [README](https://github.com/ConnectAI-E/MidJourney-Web/blob/main/README.md)有 MJ 图片操作与进度界面，但 README 自称施工进度 80%。已检查的 [src/components/SendArea.tsx](https://github.com/ConnectAI-E/MidJourney-Web/blob/main/src/components/SendArea.tsx) 以提示词 textarea 和图片上传为主，[Generator.tsx](https://github.com/ConnectAI-E/MidJourney-Web/blob/main/src/components/Generator.tsx)处理异步生成/图片操作，没有本任务所需的 stylize、iw、版本专用面板证据，因此不作为主要推荐。[仓库 API](https://api.github.com/repos/ConnectAI-E/MidJourney-Web)显示最近 push 为 2024-01-20，license 未识别；后续若复用须先核实授权。

## 对当前产品的直接参考价值

Dooy 可用于参考“比例可视化按钮、风格化档位、上传参考图、进阶设置折叠”的现成界面；ChatAny 可用于参考“纯提示词输入与参数配置分离、单独 `iw` 控件、声明式字段序列化”。V8.2 和 Niji V7 的实际参数范围、互斥关系及默认值仍应以最新官方文档和本渠道实测为准；本轮只核实前端实现，不声称这两个开源项目已经验证新版本生成。
