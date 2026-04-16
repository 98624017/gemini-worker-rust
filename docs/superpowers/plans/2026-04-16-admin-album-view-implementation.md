# /admin Album View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `/admin` 页面新增默认折叠的图表区与精致的相册视角，并支持从相册跳回现有列表中的对应日志记录。

**Architecture:** 继续沿用 `src/admin.rs` 中的单文件 HTML/CSS/JS 方案，不新增后端 API。前端将现有“先渲染列表再 DOM 隐藏过滤”的流程改为“先根据状态得到 filteredItems，再按 `viewMode` 渲染列表或相册”，这样两种视图能共享搜索、状态筛选和 finishReason 筛选。列表项与相册卡片都绑定 `item.id`，由 `jumpToListItem(id)` 负责跨视图定位与展开。

**Tech Stack:** Rust、Axum、内嵌 HTML/CSS/Vanilla JS、Cargo test

---

## File Structure

- Modify: `src/admin.rs`
  - 增加图表折叠容器、视图切换器、相册容器
  - 增加相册视图 CSS
  - 增加 `viewMode` / `chartsCollapsed` / `pendingScrollTargetId` 等前端状态
  - 把过滤逻辑改为基于数据重渲染
  - 增加 prompt 提取、相册卡片渲染、跳回列表定位逻辑
- Modify: `tests/admin_test.rs`
  - 为新 UI 结构和关键 JS 函数增加 HTML 字符串级回归测试

---

### Task 1: 锁定折叠图表与视图切换骨架

**Files:**
- Modify: `tests/admin_test.rs`
- Modify: `src/admin.rs`
- Test: `tests/admin_test.rs`

- [ ] **Step 1: 写出失败测试，固定图表折叠容器和视图切换器的存在**

```rust
#[tokio::test]
async fn admin_logs_page_contains_chart_collapse_and_view_switch() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("chartsSection"),
        "HTML should contain charts section wrapper"
    );
    assert!(
        html.contains("chartsToggle"),
        "HTML should contain charts collapse toggle"
    );
    assert!(
        html.contains("viewModeTabs"),
        "HTML should contain view mode switch tabs"
    );
    assert!(
        html.contains("data-view=\"album\""),
        "HTML should contain album view switch"
    );
}

#[tokio::test]
async fn admin_logs_page_persists_view_mode_and_chart_collapse_state() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("admin:viewMode"),
        "HTML should persist selected admin view mode"
    );
    assert!(
        html.contains("admin:chartsCollapsed"),
        "HTML should persist chart collapse state"
    );
    assert!(
        html.contains("function setChartsCollapsed("),
        "HTML should expose chart collapse state setter"
    );
    assert!(
        html.contains("function setViewMode("),
        "HTML should expose view mode state setter"
    );
}
```

- [ ] **Step 2: 运行测试，确认当前实现还没有这些结构**

Run:

```bash
timeout 60 cargo test admin_logs_page_contains_chart_collapse_and_view_switch --test admin_test -- --exact
timeout 60 cargo test admin_logs_page_persists_view_mode_and_chart_collapse_state --test admin_test -- --exact
```

Expected:

```text
FAILED tests::admin_logs_page_contains_chart_collapse_and_view_switch
FAILED tests::admin_logs_page_persists_view_mode_and_chart_collapse_state
```

- [ ] **Step 3: 在 `src/admin.rs` 中加入图表折叠容器和视图切换器骨架**

```html
  <div class="toolbar">
    <div class="filter-tabs">
      <button class="filter-tab active" data-filter="all">All</button>
      <button class="filter-tab" data-filter="ok">2xx</button>
      <button class="filter-tab" data-filter="bad">4xx+</button>
    </div>
    <div class="view-mode-tabs" id="viewModeTabs" role="tablist" aria-label="content view mode">
      <button class="view-mode-tab active" data-view="list" aria-pressed="true">列表视图</button>
      <button class="view-mode-tab" data-view="album" aria-pressed="false">相册视图</button>
    </div>
    <input class="search" id="searchBox" type="search" placeholder="search path / model..." />
    <span class="count-badge" id="countBadge"></span>
    <span class="status-line" id="statusLine"></span>
  </div>

  <section class="charts-section collapsed" id="chartsSection">
    <button class="charts-toggle" id="chartsToggle" type="button" aria-expanded="false">
      <span class="charts-toggle-copy">
        <strong>趋势图表</strong>
        <span>4 个趋势图，按需展开</span>
      </span>
      <span class="charts-toggle-icon" id="chartsToggleIcon"></span>
    </button>
    <div class="charts-panel" id="chartsPanel">
      <div class="charts" id="chartsRow">
        <!-- existing chart cards -->
      </div>
    </div>
  </section>
```

- [ ] **Step 4: 加入对应的样式和状态读写函数**

```javascript
  var STORAGE_KEYS = {
    theme: 'theme',
    viewMode: 'admin:viewMode',
    chartsCollapsed: 'admin:chartsCollapsed'
  };

  function readViewMode() {
    var stored = localStorage.getItem(STORAGE_KEYS.viewMode);
    return stored === 'album' ? 'album' : 'list';
  }

  function readChartsCollapsed() {
    var stored = localStorage.getItem(STORAGE_KEYS.chartsCollapsed);
    return stored === null ? true : stored === 'true';
  }

  function setViewMode(mode, persist) {
    viewMode = mode === 'album' ? 'album' : 'list';
    if (persist) localStorage.setItem(STORAGE_KEYS.viewMode, viewMode);
  }

  function setChartsCollapsed(collapsed, persist) {
    chartsCollapsed = !!collapsed;
    if (persist) localStorage.setItem(STORAGE_KEYS.chartsCollapsed, String(chartsCollapsed));
    chartsSection.classList.toggle('collapsed', chartsCollapsed);
    chartsToggle.setAttribute('aria-expanded', String(!chartsCollapsed));
  }
```

```css
    .view-mode-tabs { display: flex; background: var(--bg-input); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; }
    .view-mode-tab { min-height: 34px; padding: 6px 14px; border: 0; background: transparent; color: var(--text-secondary); cursor: pointer; }
    .view-mode-tab.active { background: var(--bg-card-hover); color: var(--text-primary); font-weight: 600; }
    .charts-section { margin-bottom: 20px; }
    .charts-toggle { width: 100%; min-height: 52px; display: flex; align-items: center; justify-content: space-between; padding: 12px 16px; border: 1px solid var(--border); border-radius: 14px; background: var(--bg-card); color: var(--text-primary); cursor: pointer; }
    .charts-panel { max-height: 960px; opacity: 1; overflow: hidden; transition: max-height 0.18s ease, opacity 0.18s ease, margin-top 0.18s ease; margin-top: 12px; }
    .charts-section.collapsed .charts-panel { max-height: 0; opacity: 0; margin-top: 0; }
```

- [ ] **Step 5: 绑定初始化和交互，但先不改主渲染流程**

```javascript
  var viewMode = readViewMode();
  var chartsCollapsed = readChartsCollapsed();
  var pendingScrollTargetId = null;

  var chartsSection = document.getElementById('chartsSection');
  var chartsToggle = document.getElementById('chartsToggle');
  var chartsPanel = document.getElementById('chartsPanel');

  document.querySelectorAll('.view-mode-tab').forEach(function (btn) {
    btn.addEventListener('click', function () {
      setViewMode(btn.dataset.view, true);
      document.querySelectorAll('.view-mode-tab').forEach(function (node) {
        var active = node.dataset.view === viewMode;
        node.classList.toggle('active', active);
        node.setAttribute('aria-pressed', String(active));
      });
    });
  });

  chartsToggle.addEventListener('click', function () {
    setChartsCollapsed(!chartsCollapsed, true);
  });

  setChartsCollapsed(chartsCollapsed, false);
```

- [ ] **Step 6: 运行测试，确认骨架已经生效**

Run:

```bash
timeout 60 cargo test admin_logs_page_contains_chart_collapse_and_view_switch --test admin_test -- --exact
timeout 60 cargo test admin_logs_page_persists_view_mode_and_chart_collapse_state --test admin_test -- --exact
```

Expected:

```text
test admin_logs_page_contains_chart_collapse_and_view_switch ... ok
test admin_logs_page_persists_view_mode_and_chart_collapse_state ... ok
```

- [ ] **Step 7: 提交当前骨架**

```bash
git add src/admin.rs tests/admin_test.rs
git commit -m "feat: add admin view mode and chart collapse shell"
```

---

### Task 2: 把筛选改为共享数据流，并接通列表 / 相册双容器

**Files:**
- Modify: `tests/admin_test.rs`
- Modify: `src/admin.rs`
- Test: `tests/admin_test.rs`

- [ ] **Step 1: 写出失败测试，锁定共享过滤函数与双容器渲染**

```rust
#[tokio::test]
async fn admin_logs_page_uses_shared_filtered_items_for_all_views() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("function getFilteredItems()"),
        "HTML should compute filtered items once for all views"
    );
    assert!(
        html.contains("function renderMainContent(items)"),
        "HTML should render current content from a shared pipeline"
    );
    assert!(
        html.contains("albumList"),
        "HTML should contain dedicated album container"
    );
}
```

- [ ] **Step 2: 运行测试，确认当前还没有共享渲染管线**

Run:

```bash
timeout 60 cargo test admin_logs_page_uses_shared_filtered_items_for_all_views --test admin_test -- --exact
```

Expected:

```text
FAILED tests::admin_logs_page_uses_shared_filtered_items_for_all_views
```

- [ ] **Step 3: 在 DOM 中新增相册容器，并保留列表容器**

```html
  <div id="frBar" class="fr-bar"></div>

  <div class="content-stack" id="contentStack">
    <div id="logList"></div>
    <div id="albumList" class="album-list hidden" aria-live="polite"></div>
  </div>
```

```css
    .hidden { display: none !important; }
    .content-stack { min-height: 120px; }
```

- [ ] **Step 4: 把过滤逻辑改成数据驱动，而不是直接隐藏 DOM 节点**

```javascript
  function matchesFilters(item) {
    var model = extractModel(item.path);
    var fr = (item.finishReason || '').toUpperCase();
    var q = searchText.toLowerCase();
    var isOk = item.statusCode >= 200 && item.statusCode < 400;
    var matchFilter = filterMode === 'all'
      || (filterMode === 'ok' && isOk)
      || (filterMode === 'bad' && !isOk);
    var matchFr = finishReasonFilter === 'all' || fr === finishReasonFilter;
    var haystack = (model + ' ' + item.path + ' ' + item.statusCode + ' ' + fr).toLowerCase();
    var matchSearch = !q || haystack.includes(q);
    return matchFilter && matchFr && matchSearch;
  }

  function getFilteredItems() {
    return allItems.filter(matchesFilters);
  }

  function updateCount(shown, total) {
    elCount.textContent = shown === total ? shown + ' total' : shown + ' / ' + total;
  }
```

- [ ] **Step 5: 拆出列表渲染函数和主渲染入口**

```javascript
  function renderList(items) {
    elList.innerHTML = '';
    if (!items.length) {
      elList.innerHTML = '<div class="empty"><svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" opacity="0.3"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><line x1="9" y1="21" x2="9" y2="9"/></svg><p>no matching requests</p></div>';
      return;
    }
    var frag = document.createDocumentFragment();
    items.forEach(function (it) { frag.appendChild(buildRow(it)); });
    elList.appendChild(frag);
  }

  function renderAlbum(items) {
    if (!items.length) {
      elAlbum.innerHTML = '<div class="empty"><p>no matching requests</p></div>';
      return;
    }
    elAlbum.innerHTML = items.map(function (item) {
      return '<article class="album-card" data-item-id="' + esc(item.id) + '">'
        + '<div class="album-head"><strong>#' + esc(item.id) + '</strong><span class="log-dur">' + fmtDur(item.durationMs) + '</span></div>'
        + '<div class="album-prompt"><pre>' + esc(truncateText(item.requestRaw || '', 160)) + '</pre></div>'
        + '</article>';
    }).join('');
  }

  function renderMainContent(items) {
    var isAlbum = viewMode === 'album';
    elList.classList.toggle('hidden', isAlbum);
    elAlbum.classList.toggle('hidden', !isAlbum);
    if (isAlbum) renderAlbum(items);
    else renderList(items);
  }
```

- [ ] **Step 6: 改写 `loadLogs()` 和筛选交互，使其统一调用主渲染入口**

```javascript
  var elAlbum = document.getElementById('albumList');

  function rerenderContent() {
    var filtered = getFilteredItems();
    renderMainContent(filtered);
    updateCount(filtered.length, allItems.length);
  }

  function loadLogs() {
    elStatus.textContent = 'loading...';
    return fetch(apiBase + '/admin/api/logs', { cache: 'no-store' })
      .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); })
      .then(function (d) {
        allItems = (d && d.items) || [];
        rebuildFrBar();
        renderCharts(allItems);
        rerenderContent();
        elStatus.textContent = 'updated ' + new Date().toLocaleTimeString('zh-CN');
      })
      .catch(function (e) {
        elStatus.textContent = 'load failed: ' + e.message;
      });
  }
```

- [ ] **Step 7: 运行测试，确认共享数据流骨架生效**

Run:

```bash
timeout 60 cargo test admin_logs_page_uses_shared_filtered_items_for_all_views --test admin_test -- --exact
```

Expected:

```text
test admin_logs_page_uses_shared_filtered_items_for_all_views ... ok
```

- [ ] **Step 8: 提交共享渲染改造**

```bash
git add src/admin.rs tests/admin_test.rs
git commit -m "refactor: share filtered admin content across views"
```

---

### Task 3: 实现相册卡片、prompt 提取和结果图主视觉

**Files:**
- Modify: `tests/admin_test.rs`
- Modify: `src/admin.rs`
- Test: `tests/admin_test.rs`

- [ ] **Step 1: 写出失败测试，固定 prompt 提取和相册卡片逻辑的存在**

```rust
#[tokio::test]
async fn admin_logs_page_contains_album_rendering_helpers() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("function extractPromptText(item)"),
        "HTML should extract prompt text for album cards"
    );
    assert!(
        html.contains("function renderAlbum(items)"),
        "HTML should render album cards"
    );
    assert!(
        html.contains("album-card"),
        "HTML should style album cards"
    );
    assert!(
        html.contains("查看对应记录"),
        "HTML should expose jump back action from album"
    );
}
```

- [ ] **Step 2: 运行测试，确认当前还没有相册渲染**

Run:

```bash
timeout 60 cargo test admin_logs_page_contains_album_rendering_helpers --test admin_test -- --exact
```

Expected:

```text
FAILED tests::admin_logs_page_contains_album_rendering_helpers
```

- [ ] **Step 3: 增加 prompt 提取和摘要退化逻辑**

```javascript
  function truncateText(text, max) {
    if (!text) return '';
    return text.length > max ? text.slice(0, max) + '…' : text;
  }

  function extractPromptText(item) {
    var raw = item && item.requestRaw ? item.requestRaw : '';
    if (!raw) return '';
    try {
      var parsed = JSON.parse(raw);
      var lines = [];
      (parsed.contents || []).forEach(function (content) {
        (content.parts || []).forEach(function (part) {
          if (part && typeof part.text === 'string' && part.text.trim()) {
            lines.push(part.text.trim());
          }
        });
      });
      if (lines.length) return lines.join('\n');
    } catch (_) {}
    return truncateText(raw, 280);
  }
```

- [ ] **Step 4: 增加相册所需 CSS，采用深色胶片画廊风格**

```css
    .album-list { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; }
    @media (max-width: 980px) { .album-list { grid-template-columns: 1fr; } }
    .album-card { position: relative; overflow: hidden; border-radius: 22px; border: 1px solid var(--border); background: linear-gradient(180deg, rgba(12,18,31,0.98), rgba(8,14,31,0.92)); box-shadow: 0 18px 40px rgba(0,0,0,0.18); transition: transform 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease; }
    .album-card:hover { transform: translateY(-2px); border-color: var(--border-hover); box-shadow: 0 24px 50px rgba(0,0,0,0.22); }
    .album-head { display: flex; justify-content: space-between; gap: 12px; align-items: flex-start; padding: 16px 18px 0; }
    .album-prompt { margin: 14px 18px 0; padding: 14px 16px; border-radius: 18px; background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.08); }
    .album-grid { display: grid; grid-template-columns: minmax(0, 0.95fr) minmax(0, 1.25fr); gap: 14px; padding: 14px 18px 18px; }
    .album-inputs { display: grid; gap: 10px; }
    .album-input-thumbs { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
    .album-thumb img, .album-result img { width: 100%; display: block; object-fit: cover; }
    .album-result { border-radius: 20px; overflow: hidden; border: 1px solid rgba(255,255,255,0.08); background: rgba(255,255,255,0.03); min-height: 280px; }
```

- [ ] **Step 5: 增加相册卡片渲染函数，让结果图成为主视觉**

```javascript
  function renderAlbumThumb(url, cacheHit, alt) {
    var safe = safeUrl(url);
    if (!safe) return '';
    var badge = cacheHit ? '<span class="cache-badge">CACHE</span>' : '';
    return '<a class="album-thumb" href="' + esc(safe) + '" target="_blank" rel="noreferrer">'
      + badge
      + '<img src="' + esc(safe) + '" alt="' + esc(alt) + '" loading="lazy">'
      + '</a>';
  }

  function renderAlbumCard(item) {
    var prompt = extractPromptText(item);
    var promptPreview = truncateText(prompt, 220);
    var promptHtml = prompt
      ? '<div class="album-prompt"><div class="detail-col-label">prompt</div><pre>' + esc(promptPreview) + '</pre></div>'
      : '';
    var hitSet = {};
    (item.requestRawImageCacheHits || []).forEach(function (hit) { hitSet[hit] = true; });
    var requestThumbs = (item.requestRawImages || []).map(function (url, idx) {
      return renderAlbumThumb(url, !!hitSet[url], '请求图片 ' + (idx + 1));
    }).join('');
    var resultUrl = (item.responseImages || [])[0];
    var resultHtml = resultUrl
      ? '<a class="album-result" href="' + esc(resultUrl) + '" target="_blank" rel="noreferrer"><img src="' + esc(resultUrl) + '" alt="生成结果图" loading="lazy"></a>'
      : '<div class="album-result empty-result"><div class="detail-col-label">result</div><p>暂无结果图</p></div>';

    return '<article class="album-card" data-item-id="' + esc(item.id) + '">'
      + '<div class="album-head"><div><strong>#' + esc(item.id) + '</strong><div class="stat-sub">' + esc(new Date(item.createdAt).toLocaleString('zh-CN')) + '</div></div>'
      + '<div class="log-meta">' + buildAlbumTags(item) + '<span class="log-dur">' + fmtDur(item.durationMs) + '</span></div></div>'
      + promptHtml
      + '<div class="album-grid"><div class="album-inputs"><div class="detail-col-label">request images</div><div class="album-input-thumbs">' + requestThumbs + '</div></div>'
      + '<div><div class="detail-col-label">result</div>' + resultHtml + '</div></div>'
      + '<div class="album-actions"><button class="btn jump-to-log-btn" data-log-id="' + esc(item.id) + '">查看对应记录</button></div>'
      + '</article>';
  }

  function renderAlbum(items) {
    elAlbum.innerHTML = '';
    if (!items.length) {
      elAlbum.innerHTML = '<div class="empty"><p>no matching requests</p></div>';
      return;
    }
    elAlbum.innerHTML = items.map(renderAlbumCard).join('');
  }
```

- [ ] **Step 6: 增加补充辅助函数，让相册标签和错误退化更自然**

```javascript
  function buildAlbumTags(item) {
    var isOk = item.statusCode >= 200 && item.statusCode < 400;
    var parts = [
      isOk ? '<span class="tag tag-ok">' + esc(item.statusCode) + '</span>' : '<span class="tag tag-bad">' + esc(item.statusCode) + '</span>'
    ];
    if (item.isStream) parts.push('<span class="tag tag-stream">stream</span>');
    if (item.finishReason) parts.push('<span class="tag tag-fr">' + esc(String(item.finishReason).toUpperCase()) + '</span>');
    if ((item.durationMs || 0) > 5000) parts.push('<span class="tag tag-slow">slow</span>');
    return parts.join('');
  }
```

- [ ] **Step 7: 运行测试，确认相册渲染入口和 prompt 提取代码已在页面中**

Run:

```bash
timeout 60 cargo test admin_logs_page_contains_album_rendering_helpers --test admin_test -- --exact
```

Expected:

```text
test admin_logs_page_contains_album_rendering_helpers ... ok
```

- [ ] **Step 8: 提交相册视角实现**

```bash
git add src/admin.rs tests/admin_test.rs
git commit -m "feat: add admin album gallery view"
```

---

### Task 4: 实现从相册跳回列表定位，并完成回归验证

**Files:**
- Modify: `tests/admin_test.rs`
- Modify: `src/admin.rs`
- Test: `tests/admin_test.rs`

- [ ] **Step 1: 写出失败测试，固定跨视图跳回函数和列表锚点**

```rust
#[tokio::test]
async fn admin_logs_page_contains_album_jump_back_logic() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("function jumpToListItem(itemId)"),
        "HTML should expose jump-back helper from album to list"
    );
    assert!(
        html.contains("data-item-id"),
        "HTML should stamp stable item ids on rendered elements"
    );
    assert!(
        html.contains("pendingScrollTargetId"),
        "HTML should keep pending list jump state"
    );
}
```

- [ ] **Step 2: 运行测试，确认当前还没有跳回定位逻辑**

Run:

```bash
timeout 60 cargo test admin_logs_page_contains_album_jump_back_logic --test admin_test -- --exact
```

Expected:

```text
FAILED tests::admin_logs_page_contains_album_jump_back_logic
```

- [ ] **Step 3: 给列表项增加稳定锚点，并抽出展开逻辑**

```javascript
  function openLogItem(el) {
    var item = el.__item || null;
    if (!item) return;
    var d = el.querySelector('.log-detail');
    if (!d.dataset.rendered) {
      d.innerHTML = buildDetailMarkup(item);
      d.dataset.rendered = '1';
    }
    d.classList.add('open');
    el.classList.add('expanded');
  }

  function buildRow(item) {
    var model  = extractModel(item.path);
    var isOk   = item.statusCode >= 200 && item.statusCode < 400;
    var fr = (item.finishReason || '').toUpperCase();
    var row = '<div class="log-row">'
      + '<span class="log-id">#'+esc(item.id)+'</span>'
      + '<span class="log-model" title="'+esc(item.path)+'">'+esc(model)+'</span>'
      + '<span class="log-meta">'
      + (isOk ? '<span class="tag tag-ok">'+esc(item.statusCode)+'</span>' : '<span class="tag tag-bad">'+esc(item.statusCode)+'</span>')
      + '<span class="log-dur">'+fmtDur(item.durationMs)+'</span>'
      + '</span>'
      + '</div>';
    var el = document.createElement('div');
    el.className = 'log-item';
    el.dataset.itemId = String(item.id);
    el.id = 'log-item-' + item.id;
    el.__item = item;
    el.dataset.status = isOk ? 'ok' : 'bad';
    el.dataset.fr = fr;
    el.dataset.search = (model + ' ' + item.path + ' ' + item.statusCode + ' ' + fr).toLowerCase();
    el.innerHTML = row + '<div class="log-detail"></div>';
    el.querySelector('.log-row').addEventListener('click', function () {
      var d = el.querySelector('.log-detail');
      if (d.classList.contains('open')) {
        d.classList.remove('open');
        el.classList.remove('expanded');
      } else {
        if (!d.dataset.rendered) {
          d.innerHTML = buildDetailMarkup(item);
          d.dataset.rendered = '1';
        }
        d.classList.add('open');
        el.classList.add('expanded');
      }
    });
    return el;
  }
```

- [ ] **Step 4: 实现相册跳回列表并展开目标项**

```javascript
  function jumpToListItem(itemId) {
    pendingScrollTargetId = String(itemId);
    setViewMode('list', true);
    rerenderContent();
  }

  function flushPendingScrollTarget() {
    if (!pendingScrollTargetId) return;
    var target = document.querySelector('.log-item[data-item-id="' + pendingScrollTargetId + '"]');
    if (!target) {
      console.warn('[admin] target log item is filtered out:', pendingScrollTargetId);
      pendingScrollTargetId = null;
      return;
    }
    openLogItem(target);
    target.classList.add('flash-target');
    target.scrollIntoView({ block: 'center', behavior: 'smooth' });
    setTimeout(function () { target.classList.remove('flash-target'); }, 1500);
    pendingScrollTargetId = null;
  }

  function renderMainContent(items) {
    var isAlbum = viewMode === 'album';
    elList.classList.toggle('hidden', isAlbum);
    elAlbum.classList.toggle('hidden', !isAlbum);
    if (isAlbum) renderAlbum(items);
    else renderList(items);
    if (!isAlbum) flushPendingScrollTarget();
  }

  elAlbum.addEventListener('click', function (e) {
    var button = e.target.closest('.jump-to-log-btn');
    if (!button) return;
    jumpToListItem(button.dataset.logId);
  });
```

```css
    .flash-target { box-shadow: 0 0 0 1px rgba(59,130,246,0.45), 0 0 0 8px rgba(59,130,246,0.08); }
    @media (prefers-reduced-motion: reduce) {
      .charts-panel,
      .album-card,
      .log-item,
      .flash-target { transition: none !important; animation: none !important; }
    }
```

- [ ] **Step 5: 跑新增测试和 admin 页面回归测试**

Run:

```bash
timeout 60 cargo test admin_logs_page_contains_album_jump_back_logic --test admin_test -- --exact
timeout 60 cargo test admin_logs_page_contains_chartjs_and_theme_toggle --test admin_test -- --exact
timeout 60 cargo test admin_logs_page_only_previews_proxy_images --test admin_test -- --exact
timeout 60 cargo test admin_logs_page_lazy_renders_log_details --test admin_test -- --exact
timeout 60 cargo test admin_logs_page_preserves_system_theme_without_override --test admin_test -- --exact
timeout 60 cargo test admin_logs_page_contains_error_detail_section --test admin_test -- --exact
```

Expected:

```text
test admin_logs_page_contains_album_jump_back_logic ... ok
test admin_logs_page_contains_chartjs_and_theme_toggle ... ok
test admin_logs_page_only_previews_proxy_images ... ok
test admin_logs_page_lazy_renders_log_details ... ok
test admin_logs_page_preserves_system_theme_without_override ... ok
test admin_logs_page_contains_error_detail_section ... ok
```

- [ ] **Step 6: 跑完整 `admin_test` 测试文件**

Run:

```bash
timeout 60 cargo test --test admin_test
```

Expected:

```text
test result: ok.
```

- [ ] **Step 7: 提交最终实现**

```bash
git add src/admin.rs tests/admin_test.rs
git commit -m "feat: add admin album jump-back flow"
```

---

## Self-Review Checklist

- Spec coverage:
  - 图表默认折叠：Task 1
  - 视图切换与持久化：Task 1
  - 列表 / 相册共享筛选：Task 2
  - prompt 提取与相册卡片：Task 3
  - 相册跳回列表并展开定位：Task 4
  - 响应式与 reduced motion：Task 3、Task 4
- Placeholder scan:
  - 无 `TODO` / `TBD`
  - 每个代码步骤都给了具体函数名、DOM id、命令和提交信息
- Type consistency:
  - 统一使用 `viewMode`、`chartsCollapsed`、`pendingScrollTargetId`
  - 统一使用 `data-item-id` 作为列表与相册之间的稳定锚点
