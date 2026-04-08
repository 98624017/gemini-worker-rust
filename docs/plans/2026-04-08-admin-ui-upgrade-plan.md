# Admin UI 全面升级实施计划

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在零服务端开销的前提下，全面升级 admin 后台管理界面的主题系统、数据可视化、交互体验和视觉质感。

**Architecture:** 所有改动集中在 `src/admin.rs` 的 `ADMIN_LOGS_HTML` 常量。纯 HTML/CSS/JS 客户端改造，Chart.js v4 minimal 内联。Rust 后端代码不变，API 接口不变。

**Tech Stack:** HTML5, CSS3 (CSS Variables, backdrop-filter, transitions), Chart.js v4 (inline), Vanilla JS

---

## 改动范围

- **唯一改动文件：** `src/admin.rs` 中的 `ADMIN_LOGS_HTML` 常量
- **不改动：** Rust 后端逻辑、API 端点、路由、测试
- **验证方式：** `cargo build` 编译通过 + `cargo test admin` 全部通过 + 浏览器手动验证

---

### Task 1: CSS 变量体系与双主题基础

**Files:**
- Modify: `src/admin.rs` — `ADMIN_LOGS_HTML` 常量的 `<style>` 部分

**Step 1: 替换所有硬编码颜色为 CSS 变量**

在 `<style>` 开头，将现有的 `:root { color-scheme: dark; }` 替换为完整的变量体系：

```css
/* ── Theme Variables ── */
:root {
  color-scheme: dark light;
  /* Default: dark */
  --bg-primary: #080e1f;
  --bg-secondary: rgba(8,14,31,0.85);
  --bg-card: rgba(255,255,255,0.03);
  --bg-card-hover: rgba(255,255,255,0.05);
  --bg-input: rgba(255,255,255,0.05);
  --bg-code: rgba(0,0,0,0.30);
  --bg-tag: rgba(255,255,255,0.04);
  --border: rgba(255,255,255,0.07);
  --border-hover: rgba(255,255,255,0.12);
  --border-focus: rgba(59,130,246,0.5);
  --text-primary: #e2e8f0;
  --text-secondary: rgba(226,232,240,0.6);
  --text-muted: rgba(226,232,240,0.35);
  --text-code: #cbd5e1;
  --card-gradient: linear-gradient(135deg, rgba(255,255,255,0.02) 0%, transparent 60%);
  --card-shadow: none;
  --card-blur: blur(8px);
  --scrollbar-thumb: rgba(255,255,255,0.12);
  --accent-blue: #3b82f6;
  --accent-green: #22c55e;
  --accent-amber: #f59e0b;
  --accent-red: #ef4444;
  --accent-purple: #a855f7;
  --accent-cyan: #06b6d4;
  --tag-ok-bg: rgba(34,197,94,0.10);
  --tag-ok-border: rgba(34,197,94,0.20);
  --tag-ok-text: #86efac;
  --tag-bad-bg: rgba(239,68,68,0.10);
  --tag-bad-border: rgba(239,68,68,0.20);
  --tag-bad-text: #fca5a5;
  --tag-stream-bg: rgba(59,130,246,0.08);
  --tag-stream-border: rgba(59,130,246,0.15);
  --tag-stream-text: #93c5fd;
  --tag-url-bg: rgba(168,85,247,0.08);
  --tag-url-border: rgba(168,85,247,0.15);
  --tag-url-text: #d8b4fe;
  --tag-fr-bg: rgba(99,102,241,0.08);
  --tag-fr-border: rgba(99,102,241,0.18);
  --tag-fr-text: #a5b4fc;
  --tag-cache-bg: rgba(6,182,212,0.10);
  --tag-cache-border: rgba(6,182,212,0.20);
  --tag-cache-text: #67e8f9;
  --tag-slow-bg: rgba(245,158,11,0.10);
  --tag-slow-border: rgba(245,158,11,0.20);
  --tag-slow-text: #fcd34d;
}

[data-theme="light"],
:root:not([data-theme="dark"]) {
  @media (prefers-color-scheme: light) {
    --bg-primary: #f8fafc;
    --bg-secondary: rgba(248,250,252,0.90);
    --bg-card: #ffffff;
    --bg-card-hover: #f8fafc;
    --bg-input: #f1f5f9;
    --bg-code: #f1f5f9;
    --bg-tag: rgba(0,0,0,0.03);
    --border: #e2e8f0;
    --border-hover: #cbd5e1;
    --border-focus: rgba(59,130,246,0.5);
    --text-primary: #1e293b;
    --text-secondary: #64748b;
    --text-muted: #94a3b8;
    --text-code: #475569;
    --card-gradient: linear-gradient(135deg, rgba(255,255,255,0.5) 0%, transparent 60%);
    --card-shadow: 0 1px 3px rgba(0,0,0,0.08);
    --card-blur: none;
    --scrollbar-thumb: rgba(0,0,0,0.15);
    --tag-ok-bg: rgba(34,197,94,0.08);
    --tag-ok-border: rgba(34,197,94,0.25);
    --tag-ok-text: #16a34a;
    --tag-bad-bg: rgba(239,68,68,0.08);
    --tag-bad-border: rgba(239,68,68,0.25);
    --tag-bad-text: #dc2626;
    --tag-stream-bg: rgba(59,130,246,0.06);
    --tag-stream-border: rgba(59,130,246,0.20);
    --tag-stream-text: #2563eb;
    --tag-url-bg: rgba(168,85,247,0.06);
    --tag-url-border: rgba(168,85,247,0.20);
    --tag-url-text: #9333ea;
    --tag-fr-bg: rgba(99,102,241,0.06);
    --tag-fr-border: rgba(99,102,241,0.20);
    --tag-fr-text: #6366f1;
    --tag-cache-bg: rgba(6,182,212,0.08);
    --tag-cache-border: rgba(6,182,212,0.25);
    --tag-cache-text: #0891b2;
    --tag-slow-bg: rgba(245,158,11,0.08);
    --tag-slow-border: rgba(245,158,11,0.25);
    --tag-slow-text: #d97706;
  }
}

[data-theme="light"] {
  --bg-primary: #f8fafc;
  --bg-secondary: rgba(248,250,252,0.90);
  --bg-card: #ffffff;
  --bg-card-hover: #f8fafc;
  --bg-input: #f1f5f9;
  --bg-code: #f1f5f9;
  --bg-tag: rgba(0,0,0,0.03);
  --border: #e2e8f0;
  --border-hover: #cbd5e1;
  --border-focus: rgba(59,130,246,0.5);
  --text-primary: #1e293b;
  --text-secondary: #64748b;
  --text-muted: #94a3b8;
  --text-code: #475569;
  --card-gradient: linear-gradient(135deg, rgba(255,255,255,0.5) 0%, transparent 60%);
  --card-shadow: 0 1px 3px rgba(0,0,0,0.08);
  --card-blur: none;
  --scrollbar-thumb: rgba(0,0,0,0.15);
  --tag-ok-bg: rgba(34,197,94,0.08);
  --tag-ok-border: rgba(34,197,94,0.25);
  --tag-ok-text: #16a34a;
  --tag-bad-bg: rgba(239,68,68,0.08);
  --tag-bad-border: rgba(239,68,68,0.25);
  --tag-bad-text: #dc2626;
  --tag-stream-bg: rgba(59,130,246,0.06);
  --tag-stream-border: rgba(59,130,246,0.20);
  --tag-stream-text: #2563eb;
  --tag-url-bg: rgba(168,85,247,0.06);
  --tag-url-border: rgba(168,85,247,0.20);
  --tag-url-text: #9333ea;
  --tag-fr-bg: rgba(99,102,241,0.06);
  --tag-fr-border: rgba(99,102,241,0.20);
  --tag-fr-text: #6366f1;
  --tag-cache-bg: rgba(6,182,212,0.08);
  --tag-cache-border: rgba(6,182,212,0.25);
  --tag-cache-text: #0891b2;
  --tag-slow-bg: rgba(245,158,11,0.08);
  --tag-slow-border: rgba(245,158,11,0.25);
  --tag-slow-text: #d97706;
}
```

然后将所有 CSS 规则中的硬编码颜色替换为对应变量引用。例如：
- `body { background: #080e1f; color: #e2e8f0; }` → `body { background: var(--bg-primary); color: var(--text-primary); }`
- `header { background: rgba(8,14,31,0.85); }` → `header { background: var(--bg-secondary); }`
- 所有 `.tag-*` 类同理

**Step 2: 添加全局过渡 + 字体栈 + 滚动条**

```css
*, *::before, *::after { box-sizing: border-box; transition: background-color 0.3s, color 0.3s, border-color 0.3s, box-shadow 0.3s; }
body { font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", Arial, sans-serif; }
pre, code, .stat-value, .log-id, .log-dur { font-family: "JetBrains Mono", "Fira Code", ui-monospace, "SF Mono", Menlo, monospace; }

/* ── Scrollbar ── */
::-webkit-scrollbar { width: 6px; height: 6px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: var(--scrollbar-thumb); border-radius: 3px; }
::-webkit-scrollbar-thumb:hover { background: var(--border-hover); }
```

**Step 3: 添加主题切换 JS 逻辑**

在 `<script>` 块顶部（IIFE 内第一行）添加：

```js
// ── Theme ──────────────────────────────────────────
function getPreferredTheme() {
  var stored = localStorage.getItem('theme');
  if (stored === 'dark' || stored === 'light') return stored;
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}
function applyTheme(theme) {
  document.documentElement.setAttribute('data-theme', theme);
  localStorage.setItem('theme', theme);
  var btn = document.getElementById('themeToggle');
  if (btn) btn.innerHTML = theme === 'dark' ? SUN_SVG : MOON_SVG;
}
// SVG icons (inline, ~200 bytes each)
var SUN_SVG = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>';
var MOON_SVG = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>';

applyTheme(getPreferredTheme());
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', function (e) {
  if (!localStorage.getItem('theme')) applyTheme(e.matches ? 'dark' : 'light');
});
```

**Step 4: 在 header HTML 中添加主题切换按钮**

```html
<header>
  <svg class="logo" width="20" height="20" viewBox="0 0 100 100"><path d="M50 10 C30 10 15 30 15 55 C15 80 35 90 50 90 C65 90 85 80 85 55 C85 30 70 10 50 10Z" fill="#FBBF24"/><path d="M45 8 C42 2 50 0 52 5 C54 10 48 12 45 8Z" fill="#65A30D"/></svg>
  <h1>banana-proxy 管理后台</h1>
  <label class="auto-refresh-label">
    <span class="pulse-dot" id="pulseDot"></span>
    <span class="toggle"><input type="checkbox" id="autoRefreshChk"><span class="slider"></span></span>
    自动刷新
  </label>
  <button class="btn btn-icon" id="btnRefresh" title="刷新 (r)">
    <svg class="refresh-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
  </button>
  <button class="btn btn-icon" id="themeToggle" title="切换主题"></button>
</header>
```

**Step 5: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test admin 2>&1`
Expected: 4 个 admin 测试全部 PASS

**Step 6: Commit**

```bash
git add src/admin.rs
git commit -m "feat(admin-ui): CSS variable theme system with dark/light toggle"
```

---

### Task 2: Chart.js 内联与 4 个图表

**Files:**
- Modify: `src/admin.rs` — `ADMIN_LOGS_HTML` 常量

**Step 1: 下载 Chart.js v4 minimal 并确认体积**

Run: `curl -sL "https://cdn.jsdelivr.net/npm/chart.js@4/dist/chart.umd.min.js" -o /tmp/chart.min.js && wc -c /tmp/chart.min.js`

Expected: ~200KB 未压缩（gzip 后 ~70KB，但我们内联未压缩版本）

**Step 2: 将 Chart.js 内联到 HTML**

在 `</body>` 前、现有 `<script>` 标签前插入：

```html
<script>/* Chart.js v4 minimal - https://www.chartjs.org/ */
...此处粘贴 chart.umd.min.js 完整内容...
</script>
```

**Step 3: 添加图表区域 HTML**

在 stats `</div>` 和 toolbar `<div class="toolbar">` 之间插入：

```html
<!-- Charts -->
<div class="charts" id="chartsRow">
  <div class="chart-card">
    <div class="chart-title">请求耗时分布</div>
    <canvas id="chartDuration"></canvas>
  </div>
  <div class="chart-card">
    <div class="chart-title">模型使用占比</div>
    <canvas id="chartModels"></canvas>
  </div>
  <div class="chart-card">
    <div class="chart-title">状态码分布</div>
    <canvas id="chartStatus"></canvas>
  </div>
  <div class="chart-card">
    <div class="chart-title">请求时间线</div>
    <canvas id="chartTimeline"></canvas>
  </div>
</div>
```

**Step 4: 添加图表区域 CSS**

```css
/* ── Charts ── */
.charts { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-bottom: 20px; }
@media (max-width: 900px) { .charts { grid-template-columns: 1fr; } }
.chart-card { background: var(--bg-card); border: 1px solid var(--border); border-radius: 14px; padding: 16px; backdrop-filter: var(--card-blur); box-shadow: var(--card-shadow); position: relative; overflow: hidden; }
.chart-card::before { content: ""; position: absolute; inset: 0; background: var(--card-gradient); pointer-events: none; }
.chart-title { font-size: 11px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-secondary); margin-bottom: 12px; }
.chart-card canvas { width: 100% !important; height: 200px !important; }
```

**Step 5: 添加图表聚合 + 渲染 JS**

在 `<script>` 块中，在 `refresh()` 函数之前添加：

```js
// ── Charts ─────────────────────────────────────────
var chartInstances = {};

function getChartColors() {
  var style = getComputedStyle(document.documentElement);
  return {
    text: style.getPropertyValue('--text-secondary').trim(),
    grid: style.getPropertyValue('--border').trim(),
    blue: style.getPropertyValue('--accent-blue').trim(),
    green: style.getPropertyValue('--accent-green').trim(),
    amber: style.getPropertyValue('--accent-amber').trim(),
    red: style.getPropertyValue('--accent-red').trim(),
    purple: style.getPropertyValue('--accent-purple').trim(),
    cyan: style.getPropertyValue('--accent-cyan').trim(),
  };
}

function destroyCharts() {
  Object.keys(chartInstances).forEach(function (k) {
    if (chartInstances[k]) { chartInstances[k].destroy(); chartInstances[k] = null; }
  });
}

function renderCharts(items) {
  destroyCharts();
  if (!items || !items.length || typeof Chart === 'undefined') return;
  var c = getChartColors();
  var defaults = Chart.defaults;
  defaults.color = c.text;
  defaults.borderColor = c.grid;
  defaults.animation = { duration: 400 };
  defaults.plugins.legend.labels.boxWidth = 12;
  defaults.plugins.legend.labels.padding = 8;

  // 1. Duration distribution (bar)
  var buckets = { '0-500ms': 0, '500ms-1s': 0, '1-3s': 0, '3-5s': 0, '5-10s': 0, '10s+': 0 };
  items.forEach(function (it) {
    var d = it.durationMs || 0;
    if (d <= 500) buckets['0-500ms']++;
    else if (d <= 1000) buckets['500ms-1s']++;
    else if (d <= 3000) buckets['1-3s']++;
    else if (d <= 5000) buckets['3-5s']++;
    else if (d <= 10000) buckets['5-10s']++;
    else buckets['10s+']++;
  });
  chartInstances.duration = new Chart(document.getElementById('chartDuration'), {
    type: 'bar',
    data: {
      labels: Object.keys(buckets),
      datasets: [{ data: Object.values(buckets), backgroundColor: [c.green, c.blue, c.amber, c.purple, c.red, c.red], borderRadius: 4 }]
    },
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { display: false } }, scales: { y: { beginAtZero: true, ticks: { stepSize: 1, precision: 0 } } } }
  });

  // 2. Model usage (doughnut)
  var models = {};
  items.forEach(function (it) {
    var m = extractModel(it.path) || 'unknown';
    models[m] = (models[m] || 0) + 1;
  });
  var modelKeys = Object.keys(models).sort(function (a, b) { return models[b] - models[a]; });
  var palette = [c.blue, c.green, c.amber, c.red, c.purple, c.cyan, '#f472b6', '#a3e635', '#818cf8', '#fb923c'];
  chartInstances.models = new Chart(document.getElementById('chartModels'), {
    type: 'doughnut',
    data: {
      labels: modelKeys,
      datasets: [{ data: modelKeys.map(function (k) { return models[k]; }), backgroundColor: palette.slice(0, modelKeys.length), borderWidth: 0 }]
    },
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { position: 'right', labels: { font: { size: 11 } } } } }
  });

  // 3. Status code distribution (doughnut)
  var statuses = { '2xx': 0, '4xx': 0, '5xx': 0 };
  items.forEach(function (it) {
    var s = it.statusCode || 0;
    if (s >= 200 && s < 400) statuses['2xx']++;
    else if (s >= 400 && s < 500) statuses['4xx']++;
    else statuses['5xx']++;
  });
  chartInstances.status = new Chart(document.getElementById('chartStatus'), {
    type: 'doughnut',
    data: {
      labels: ['2xx', '4xx', '5xx'],
      datasets: [{ data: [statuses['2xx'], statuses['4xx'], statuses['5xx']], backgroundColor: [c.green, c.amber, c.red], borderWidth: 0 }]
    },
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { position: 'right', labels: { font: { size: 11 } } } } }
  });

  // 4. Request timeline (line)
  var sorted = items.slice().sort(function (a, b) { return new Date(a.createdAt) - new Date(b.createdAt); });
  chartInstances.timeline = new Chart(document.getElementById('chartTimeline'), {
    type: 'line',
    data: {
      labels: sorted.map(function (it) { return new Date(it.createdAt).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }); }),
      datasets: [{ label: '耗时 (ms)', data: sorted.map(function (it) { return it.durationMs; }), borderColor: c.blue, backgroundColor: c.blue + '20', fill: true, tension: 0.3, pointRadius: 2 }]
    },
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { display: false } }, scales: { y: { beginAtZero: true } } }
  });
}
```

然后修改 `loadLogs()` 的 `.then` 回调，在 `rebuildFrBar();` 后添加 `renderCharts(allItems);`。

同时添加 `themeToggle` 按钮的事件监听，在主题切换后调用 `renderCharts(allItems)` 刷新图表配色。

**Step 6: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test admin 2>&1`
Expected: 4 个 admin 测试全部 PASS

**Step 7: Commit**

```bash
git add src/admin.rs
git commit -m "feat(admin-ui): inline Chart.js with 4 data visualization charts"
```

---

### Task 3: 交互增强——动画、键盘导航、微交互

**Files:**
- Modify: `src/admin.rs` — `ADMIN_LOGS_HTML` 常量

**Step 1: 日志展开/折叠动画 CSS**

替换现有 `.log-detail { display: none; }` 为：

```css
.log-detail { max-height: 0; opacity: 0; overflow: hidden; padding: 0 14px; border-top: 1px solid transparent; transition: max-height 0.25s ease-out, opacity 0.2s ease-out, padding 0.25s ease-out, border-color 0.25s; }
.log-detail.open { max-height: 700px; opacity: 1; padding: 14px; border-top-color: var(--border); }
.log-item.focused { outline: 2px solid var(--accent-blue); outline-offset: -2px; }
.log-item.expanded { border-left: 3px solid var(--accent-blue); }
```

**Step 2: 更新 JS 展开逻辑**

在 `buildRow()` 中，将 `d.style.display = ...` 替换为 class toggle：

```js
el.querySelector('.log-row').addEventListener('click', function () {
  var d = el.querySelector('.log-detail');
  var isOpen = d.classList.contains('open');
  d.classList.toggle('open');
  el.classList.toggle('expanded');
  if (!isOpen) {
    // scroll into view if needed
    setTimeout(function () { el.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); }, 50);
  }
});
```

**Step 3: 键盘导航 JS**

在 IIFE 中添加（filter tabs 之后）：

```js
// ── Keyboard Navigation ──────────────────────────
var focusIndex = -1;

function getVisibleItems() {
  return Array.from(elList.querySelectorAll('.log-item')).filter(function (el) { return el.style.display !== 'none'; });
}

function setFocus(idx) {
  var visible = getVisibleItems();
  if (!visible.length) return;
  // clear old focus
  var old = elList.querySelector('.log-item.focused');
  if (old) old.classList.remove('focused');
  // clamp
  focusIndex = Math.max(0, Math.min(idx, visible.length - 1));
  visible[focusIndex].classList.add('focused');
  visible[focusIndex].scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

document.addEventListener('keydown', function (e) {
  // skip when typing in search
  if (document.activeElement === elSearch) {
    if (e.key === 'Escape') { elSearch.blur(); e.preventDefault(); }
    return;
  }
  var visible = getVisibleItems();
  switch (e.key) {
    case 'j': case 'ArrowDown': e.preventDefault(); setFocus(focusIndex + 1); break;
    case 'k': case 'ArrowUp':   e.preventDefault(); setFocus(focusIndex - 1); break;
    case 'Enter': case ' ':
      e.preventDefault();
      if (focusIndex >= 0 && focusIndex < visible.length) {
        visible[focusIndex].querySelector('.log-row').click();
      }
      break;
    case 'Escape':
      e.preventDefault();
      elList.querySelectorAll('.log-detail.open').forEach(function (d) { d.classList.remove('open'); d.closest('.log-item').classList.remove('expanded'); });
      break;
    case '/': e.preventDefault(); elSearch.focus(); break;
    case 'r': e.preventDefault(); refresh(); break;
  }
});
```

**Step 4: 数字滚动动画**

添加辅助函数：

```js
function animateValue(el, end) {
  var start = parseInt(el.textContent.replace(/[^\d]/g, '')) || 0;
  if (start === end) { el.textContent = fmtNum(end); return; }
  var duration = 300;
  var startTime = null;
  function step(ts) {
    if (!startTime) startTime = ts;
    var progress = Math.min((ts - startTime) / duration, 1);
    var eased = 1 - Math.pow(1 - progress, 3); // ease-out cubic
    el.textContent = fmtNum(Math.round(start + (end - start) * eased));
    if (progress < 1) requestAnimationFrame(step);
  }
  requestAnimationFrame(step);
}
```

在 `loadStats()` 中将 `document.getElementById('s-total').textContent = fmtNum(total);` 等替换为 `animateValue(document.getElementById('s-total'), total);`。百分比和非整数字段保持 textContent 直接赋值。

**Step 5: 刷新按钮旋转 + 呼吸脉点 CSS**

```css
.btn-icon { display: inline-flex; align-items: center; justify-content: center; padding: 6px 8px; }
.refresh-icon { transition: transform 0.6s ease; }
.btn-icon.spinning .refresh-icon { transform: rotate(360deg); }

.pulse-dot { display: none; width: 6px; height: 6px; border-radius: 50%; background: var(--accent-green); }
.pulse-dot.active { display: inline-block; animation: pulse 2s ease-in-out infinite; }
@keyframes pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.3; } }
```

在 JS 中：
- 刷新按钮 click 添加 spinning class，600ms 后移除
- 自动刷新开关关联 pulseDot 的 active class

**Step 6: 日志行 hover 增强 CSS**

```css
.log-item { transition: border-color 0.15s, transform 0.15s, box-shadow 0.15s; }
.log-item:hover { border-color: var(--border-hover); transform: translateX(2px); box-shadow: 0 0 8px rgba(59,130,246,0.08); }
```

**Step 7: 添加新 tag 类型 CSS**

```css
.tag-cache  { background: var(--tag-cache-bg);  border-color: var(--tag-cache-border);  color: var(--tag-cache-text); }
.tag-slow   { background: var(--tag-slow-bg);   border-color: var(--tag-slow-border);   color: var(--tag-slow-text); }
```

在 `buildRow()` 中，durationMs > 5000 时追加 `<span class="tag tag-slow">slow</span>`。

**Step 8: 空状态 SVG 插图**

将 `'<div class="empty">暂无请求记录</div>'` 替换为：

```html
<div class="empty">
  <svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" opacity="0.3"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><line x1="9" y1="21" x2="9" y2="9"/></svg>
  <p>暂无请求记录</p>
</div>
```

**Step 9: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test admin 2>&1`
Expected: 4 个 admin 测试全部 PASS

**Step 10: Commit**

```bash
git add src/admin.rs
git commit -m "feat(admin-ui): animations, keyboard nav, micro-interactions"
```

---

### Task 4: 视觉美化收尾

**Files:**
- Modify: `src/admin.rs` — `ADMIN_LOGS_HTML` 常量

**Step 1: 卡片玻璃拟态 CSS**

```css
.stat-card { background: var(--bg-card); border: 1px solid var(--border); backdrop-filter: var(--card-blur); box-shadow: var(--card-shadow); }
.stat-card::before { background: var(--card-gradient); }
```

**Step 2: 响应式断点完善**

```css
@media (min-width: 901px) and (max-width: 1200px) { .stats { grid-template-columns: repeat(3, 1fr); } }
@media (max-width: 900px) { .stats { grid-template-columns: repeat(2, 1fr); } .charts { grid-template-columns: 1fr; } }
@media (max-width: 480px) { .stats { grid-template-columns: 1fr; } }
```

**Step 3: tag 增加 backdrop-filter**

```css
.tag { backdrop-filter: blur(4px); }
```

**Step 4: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test admin 2>&1`
Expected: 4 个 admin 测试全部 PASS

**Step 5: Commit**

```bash
git add src/admin.rs
git commit -m "feat(admin-ui): glassmorphism cards, responsive polish, visual refinements"
```

---

### Task 5: 集成测试与最终验证

**Files:**
- Modify: `tests/admin_test.rs` — 添加 HTML 页面内容验证测试

**Step 1: 添加 HTML 页面内容测试**

在 `tests/admin_test.rs` 末尾添加：

```rust
#[tokio::test]
async fn admin_logs_page_contains_chartjs_and_theme_toggle() {
    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();

    let app = rust_sync_proxy::build_router(config);
    let auth = format!("Basic {}", STANDARD.encode("user:pw"));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/logs")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8_lossy(&body);

    // Verify Chart.js is inlined
    assert!(html.contains("Chart"), "HTML should contain inlined Chart.js");

    // Verify theme toggle exists
    assert!(html.contains("themeToggle"), "HTML should contain theme toggle button");

    // Verify CSS variables are used
    assert!(html.contains("--bg-primary"), "HTML should use CSS variables");

    // Verify keyboard navigation code exists
    assert!(html.contains("keydown"), "HTML should contain keyboard navigation");
}
```

**Step 2: 运行全部测试**

Run: `cargo test 2>&1`
Expected: 所有测试 PASS

**Step 3: 手动浏览器验证清单**

启动服务：
```bash
UPSTREAM_API_KEY="test-key" ADMIN_PASSWORD="pw" cargo run
```

浏览器打开 `http://localhost:8787/admin/logs`，验证：
- [ ] 暗色主题正确加载
- [ ] 点击主题切换按钮，亮色主题正确显示
- [ ] 刷新页面后主题持久化
- [ ] 4 个图表在有数据时正确渲染
- [ ] 图表跟随主题切换变色
- [ ] j/k 键盘导航工作
- [ ] Enter 展开/折叠有动画
- [ ] Esc 折叠所有
- [ ] / 聚焦搜索框
- [ ] r 刷新
- [ ] 自动刷新绿点脉动
- [ ] 刷新按钮旋转动画
- [ ] 移动端响应式布局（Chrome DevTools）
- [ ] 滚动条样式

**Step 4: Commit**

```bash
git add tests/admin_test.rs
git commit -m "test(admin-ui): add integration test for upgraded admin page"
```
