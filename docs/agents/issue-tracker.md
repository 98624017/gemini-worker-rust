# Issue tracker: GitHub

本仓库使用 GitHub Issues：98624017/gemini-worker-rust。工程技能读写任务前读取本文件；标签约定见同目录的 triage-labels.md。

## 操作约定

- 使用 gh CLI，并显式指定仓库 98624017/gemini-worker-rust。
- 读取任务时取得完整正文、comments、labels 和 native blocking dependencies。
- 发布任务时每个 ticket 创建一个 issue，按 blockers 优先的顺序创建；多行正文使用 --body-file，保留原始换行。
- Blocked by 正文写真实 issue 链接；同时使用 GitHub native issue dependencies 建立依赖，不以 sub-issue 关系代替依赖。添加依赖时 issue_id 使用 blocker 的数字 database ID，不是 issue number 或 node_id。
- 领取任务前确认所有 blockers 已完成。ready-for-agent 表示描述足够实现，不表示任务已经解除依赖。
- 平台确实不支持 native dependencies 时，保留正文 Blocked by 引用，并在交付中说明回退情况。
- 来源不是 existing issue 时省略 Parent；拆票不关闭或修改 parent issue。

PRs as a request surface: no.
