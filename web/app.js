'use strict';

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const escapeHtml = (value) => String(value ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const icons = {
  server: '<rect x="3" y="3" width="18" height="7" rx="2"/><rect x="3" y="14" width="18" height="7" rx="2"/><path d="M7 6.5h.01M7 17.5h.01M16 6.5h2M16 17.5h2"/>',
  monitor: '<rect x="3" y="3" width="18" height="13" rx="2"/><path d="M8 21h8M12 16v5"/>',
  folder: '<path d="M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"/>',
  activity: '<path d="M3 12h4l3-8 4 16 3-8h4"/>',
  settings: '<path d="m10 3-1 3-3 1-2 3 2 2v3l2 3 3-1 2 2 3-1 1-3 3-1v-4l-3-1-1-3-3-1-3 1Z"/><circle cx="12" cy="11" r="3"/>',
  'arrow-up-right': '<path d="M6 18 18 6M6 6h12v12"/>',
  'arrow-right': '<path d="M4 12h16m-6-6 6 6-6 6"/>',
  'chevron-right': '<path d="m9 5 7 7-7 7"/>',
  'arrow-up': '<path d="M12 20V4m-7 7 7-7 7 7"/>',
  book: '<path d="M12 5C8 2 4 3 3 4v15c3-2 6-1 9 1 3-2 6-3 9-1V4c-1-1-5-2-9 1Zm0 0v15"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
  refresh: '<path d="M20 7v5h-5M4 17v-5h5M5.5 7a7 7 0 0 1 11-2L20 9M4 15l3.5 4a7 7 0 0 0 11-2"/>',
  play: '<path d="m8 4 12 8-12 8V4Z"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="1"/>',
  terminal: '<path d="m4 5 7 7-7 7m9 0h7"/>',
  scan: '<path d="M8 3H4v4m12-4h4v4M4 17v4h4m12-4v4h-4M4 12h16"/>',
  users: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8Zm13 10v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"/>',
  shield: '<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z"/><path d="m8 12 3 3 5-6"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10h.01"/>',
  edit: '<path d="m15 4 5 5M4 20l5-1L21 7a2 2 0 0 0-5-5L4 14v6Z"/>',
  x: '<path d="m6 6 12 12M6 18 18 6"/>',
  trash: '<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7"/>',
  copy: '<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 8V3H3v13h5"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  check: '<path d="m5 12 5 5L20 7"/>',
};
const icon = (name) => `<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name] || icons.server}</svg>`;
$$('[data-icon]').forEach((el) => { el.innerHTML = icon(el.dataset.icon); });
const app = { runners: [], token: '', platform: '', config_path: '', cli: {}, cliTarget: '', cliPending: false, selected: new Set(), focused: '', inspect: null, filter: 'all', search: '', page: 'runners', logRunner: '', busy: false, loaded: false, signature: '' };
const names = { runners: 'Runner', folders: '目录', activity: '日志', settings: '设置' };
const statusText = { running: '运行中', restarting: '待重启', stopped: '已停止', exited: '已退出' };
const runnerBusy = (r) => ['running', 'restarting'].includes(r.status);
const runnerById = (id) => app.runners.find((r) => r.config.id === id);
const runnerUrl = (id) => `/api/runners/${encodeURIComponent(id)}`;
const folderRows = () => app.runners.flatMap((r) => r.config.directories.map((path) => ({ runner: r, path })));
const uniqueFolderCount = () => new Set(folderRows().map(({ runner, path }) => `${runner.config.backend}:${runner.config.distribution}:${app.platform === 'windows' && runner.config.backend === 'native' ? path.replaceAll('/', '\\').replace(/\\+$/, '').toLowerCase() : path.replace(/\/+$/, '')}`)).size;
const pill = (r) => `<span class="pill ${escapeHtml(r.status)}"><i class="dot"></i>${statusText[r.status] || '未知'}</span>`;
const button = (action, text, symbol, cls = '', attributes = '') => `<button type="button" class="btn ${cls}" data-action="${action}" ${attributes}>${symbol ? icon(symbol) : ''}${text}</button>`;
const iconButton = (action, label, symbol, attributes = '') => `<button type="button" class="icon-btn" data-action="${action}" title="${escapeHtml(label)}" aria-label="${escapeHtml(label)}" ${attributes}>${icon(symbol)}</button>`;

async function api(path, body, method = 'POST') {
  const options = body === undefined ? {} : { method, headers: { 'content-type': 'application/json', 'x-ampm-token': app.token }, body: JSON.stringify(body) };
  let response;
  try { response = await fetch(path, options); }
  catch { throw new Error('无法连接本地管理器。操作结果可能尚未返回，请恢复连接后刷新核对。'); }
  const text = await response.text();
  let result;
  try { result = JSON.parse(text); } catch { throw new Error(`服务返回了无法读取的响应（${response.status}）：${text.slice(0, 200)}`); }
  if (!response.ok) throw new Error(result.error || `请求失败（${response.status}）`);
  return result;
}
function toast(message, error = false) {
  const node = document.createElement('div');
  node.className = `toast${error ? ' error' : ''}`;
  node.textContent = message;
  $('#toasts').append(node);
  setTimeout(() => node.remove(), error ? 10000 : 4200);
}
function formError(dialog, error) {
  const node = $('.form-error', dialog);
  if (node) { node.textContent = error.message; node.hidden = false; node.scrollIntoView({ block: 'nearest' }); }
  else toast(error.message, true);
}
function confirmAction(title, text, accept = '确认') {
  const dialog = $('#confirm');
  dialog.innerHTML = `<form method="dialog"><div class="dialog-head"><h2 id="confirm-title">${escapeHtml(title)}</h2></div><div class="dialog-body"><p class="confirm-text">${escapeHtml(text)}</p></div><div class="dialog-footer"><button class="btn" value="cancel" autofocus>取消</button><button class="btn primary" value="accept">${escapeHtml(accept)}</button></div></form>`;
  dialog.returnValue = 'cancel';
  dialog.showModal();
  return new Promise((resolve) => dialog.addEventListener('close', () => resolve(dialog.returnValue === 'accept'), { once: true }));
}
async function refresh(force = false) {
  try {
    const state = await api('/api/state');
    const signature = JSON.stringify([state.runners, state.update, state.cli, state.startup]);
    Object.assign(app, { runners: state.runners, token: state.token, platform: state.platform, config_path: state.config_path, update: state.update, cli: state.cli, startup: state.startup, loaded: true });
    app.selected = new Set([...app.selected].filter((id) => runnerById(id)));
    $('#connection').className = 'connection';
    $('#connection').innerHTML = '<i></i><span>后台已连接</span>';
    $('#nav-count').textContent = app.runners.length;
    $('#folder-count').textContent = uniqueFolderCount();
    if (signature !== app.signature || force) { app.signature = signature; render(); }
    $$('[data-started]').forEach((el) => { el.textContent = uptime(Number(el.dataset.started)); });
  } catch (error) {
    $('#connection').className = 'connection offline';
    $('#connection').innerHTML = '<i></i><span>后台已断开</span>';
    if (!app.loaded) $('#main').innerHTML = `<div class="empty-state"><h2>还没有连接到管理器</h2><p>${escapeHtml(error.message)}</p>${button('refresh', '重新连接', 'refresh')}</div>`;
    if (force && app.loaded) toast(error.message, true);
  }
}
function uptime(started) {
  const minutes = Math.max(0, Math.floor((Date.now() - started) / 60000));
  return minutes < 1 ? '刚刚启动' : minutes < 60 ? `已运行 ${minutes} 分钟` : `已运行 ${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分钟`;
}
function heading(title, actions = '') {
  return `<header class="page-heading"><h1>${title}</h1><div class="heading-actions">${actions}</div></header>`;
}
function visibleRunners() {
  const query = app.search.toLowerCase();
  return app.runners.filter((r) => (app.filter === 'all' || (app.filter === 'running' ? r.status === 'running' : r.status !== 'running')) && [r.config.name, r.config.id, ...r.config.directories].some((s) => s.toLowerCase().includes(query)));
}
function emptyState(filtered = false) {
  return `<div class="empty-state">${icon(filtered ? 'search' : 'server')}<h3>${filtered ? '无匹配结果' : '尚无 Runner'}</h3>${button(filtered ? 'clear-filter' : 'new', filtered ? '清除筛选' : '新建 Runner', filtered ? '' : 'plus', filtered ? '' : 'primary')}</div>`;
}
function runnerRow(r) {
  const c = r.config, id = escapeHtml(c.id);
  return `<div class="runner-row ${app.focused === c.id ? 'focused' : ''}"><input type="checkbox" data-select="${id}" aria-label="选择 ${escapeHtml(c.name)}" ${app.selected.has(c.id) ? 'checked' : ''}><button class="runner-item" data-action="details" data-id="${id}" aria-current="${app.focused === c.id}"><span class="row-title"><strong>${escapeHtml(c.name)}</strong><i class="status-dot ${escapeHtml(r.status)}" title="${statusText[r.status]}" aria-label="${statusText[r.status]}"></i></span><span class="row-id mono">${id}</span><span class="row-sub">${c.backend === 'wsl' ? 'WSL' : '本机'} · ${c.directories.length} 个目录</span></button></div>`;
}
function runnerDetail(r) {
  if (!r) return '<div class="empty-state"><p>选择一个 Runner 查看详情</p></div>';
  const c = r.config, id = escapeHtml(c.id), running = r.status === 'running', busy = runnerBusy(r);
  const attrs = `data-id="${id}" ${app.busy ? 'disabled' : ''}`;
  return `<div class="detail-heading"><div><h2>${escapeHtml(c.name)}</h2><code>${id}</code></div>${pill(r)}</div><div class="detail-actions">${button(busy ? 'stop' : 'start', busy ? '停止' : '启动', busy ? 'stop' : 'play', busy ? '' : 'primary', attrs)}${running ? button('restart', '重启', 'refresh', '', attrs) : ''}${button('edit', '编辑', 'edit', '', `data-id="${id}" ${busy || app.busy ? 'disabled title="停止后可编辑启动配置"' : ''}`)}${button('logs', '日志', 'terminal', '', `data-id="${id}"`)}</div>${r.exit ? `<div class="notice">${escapeHtml(r.exit)}${r.status === 'restarting' ? '<br>等待自动重启，点击“停止”可取消。' : ''}</div>` : ''}<section class="detail-section"><h3>运行信息</h3><dl class="properties"><dt>环境</dt><dd>${c.backend === 'wsl' ? `WSL · ${escapeHtml(c.distribution || '默认发行版')}` : escapeHtml(app.platform === 'windows' ? 'Windows · 本机' : app.platform)}</dd><dt>进程</dt><dd>${r.pid ? `PID ${r.pid}` : '—'}${r.started ? `<span class="muted" data-started="${r.started}">${uptime(r.started)}</span>` : ''}</dd><dt>可执行文件</dt><dd><code>${escapeHtml(c.executable)}</code></dd><dt>随 AMPM 启动</dt><dd>${c.autostart ? '已开启' : '已关闭'}</dd><dt>异常自动重启</dt><dd>${c.restart_limit ? `已用 ${r.restart_attempts} / ${c.restart_limit} 次 · 间隔 5 秒` : '已关闭'}</dd><dt>自动发现</dt><dd>${c.discover ? '已开启' : '已关闭'}</dd><dt>远程终端</dt><dd>${c.terminal ? '<span class="permission-on">已开启</span>' : '已关闭'}</dd><dt>工作区共享</dt><dd>${c.share ? '<span class="permission-on">已共享</span>' : '仅自己'}</dd></dl></section><section class="detail-section"><div class="section-head"><h3>目录 <span class="muted">${c.directories.length}</span></h3>${button('add-folder', '添加', 'plus', 'subtle', `data-id="${id}"`)}</div><div class="directory-list">${c.directories.map((path) => `<div class="detail-dir">${icon('folder')}<div><code>${escapeHtml(path)}</code></div>${iconButton('remove-folder', '解除目录关联', 'trash', `data-id="${id}" data-path="${escapeHtml(path)}"`)}</div>`).join('')}</div><div class="directory-footer">${button('inspect', '读取实际目录', 'scan', 'subtle', `data-id="${id}" ${!running || app.inspect?.loading ? 'disabled' : ''} title="读取 Amp CLI 返回的目录，需 Runner 运行中"`)}<details class="inline-help"><summary>目录规则</summary><p>启动目录需停止后移除。自动发现及 Amp 记忆的目录以 CLI 输出为准；目录列表不是权限沙箱。</p></details></div><pre class="terminal-output" id="inspect-output" ${app.inspect?.id === c.id ? '' : 'hidden'}>${app.inspect?.id === c.id ? escapeHtml(app.inspect.text) : ''}</pre></section>`;
}
function renderRunnerList() {
  const container = $('#runner-list');
  if (!container) return;
  const scroll = container.scrollTop;
  const focusedControl = document.activeElement?.closest('[data-select], .runner-item, #select-all');
  const focusId = focusedControl?.dataset.select || focusedControl?.dataset.id;
  const wasCheckbox = focusedControl?.matches('[data-select]');
  const runners = visibleRunners();
  if (!runners.some((r) => r.config.id === app.focused)) app.focused = runners[0]?.config.id || '';
  container.innerHTML = runners.length ? runners.map(runnerRow).join('') : emptyState(app.runners.length > 0);
  container.scrollTop = scroll;
  $('#runner-detail').innerHTML = runnerDetail(runnerById(app.focused));
  $('#batch-toolbar').innerHTML = `<label class="select-all"><input type="checkbox" id="select-all" aria-label="选择当前列表全部 Runner" ${runners.length && runners.every((r) => app.selected.has(r.config.id)) ? 'checked' : ''} ${runners.length ? '' : 'disabled'}><span>${app.selected.size ? `已选 ${app.selected.size} 项` : '全选'}</span></label><div class="batch-actions" ${app.selected.size ? '' : 'hidden'}>${button('batch-start', '启动', 'play', 'subtle', app.busy ? 'disabled' : '')}${button('batch-stop', '停止', 'stop', 'subtle', app.busy ? 'disabled' : '')}${button('batch-restart', '重启', 'refresh', 'subtle', app.busy ? 'disabled' : '')}${button('clear-selection', '取消选择', '', 'subtle')}</div>`;
  const selectedVisible = runners.filter((r) => app.selected.has(r.config.id)).length;
  $('#select-all').indeterminate = selectedVisible > 0 && selectedVisible < runners.length;
  if (focusedControl?.id === 'select-all') $('#select-all').focus({ preventScroll: true });
  else if (focusId) $$('[data-select], .runner-item', container).find((el) => (wasCheckbox ? el.dataset.select : el.dataset.id) === focusId)?.focus({ preventScroll: true });
}
function runnersPage() {
  return `${heading('Runner', iconButton('refresh', '刷新', 'refresh') + button('external', '本机发现', 'scan') + button('new', '新建', 'plus', 'primary'))}<div id="batch-toolbar" class="batch-toolbar"></div><div class="runner-workspace"><section class="runner-master" aria-label="Runner 列表"><div class="list-tools"><label class="search">${icon('search')}<input id="search" type="search" aria-label="搜索 Runner" placeholder="搜索名称、ID 或目录" value="${escapeHtml(app.search)}"></label><div class="segmented" aria-label="状态筛选">${[['all', '全部'], ['running', '运行中'], ['stopped', '未运行']].map(([value, label]) => `<button data-filter="${value}" class="${app.filter === value ? 'active' : ''}" aria-pressed="${app.filter === value}">${label}</button>`).join('')}</div></div><div id="runner-list" data-scroll="runners"></div></section><section id="runner-detail" class="runner-detail" data-scroll="detail" aria-label="Runner 详情"></section></div>`;
}
function foldersPage() {
  const rows = folderRows();
  return `${heading('目录', button('add-folder', '添加目录', 'plus', 'primary', app.runners.length ? '' : 'disabled'))}<div class="page-content" data-scroll="folders">${rows.length ? `<table class="folder-table"><thead><tr><th>文件夹</th><th>Runner</th><th>管理</th></tr></thead><tbody>${rows.map(({ runner: r, path }) => `<tr><td><div class="folder-cell">${icon('folder')}<div><strong>${escapeHtml(path.split(/[\\/]/).filter(Boolean).pop() || path)}</strong><code>${escapeHtml(path)}</code></div></div></td><td><button class="text-link" data-action="details" data-id="${escapeHtml(r.config.id)}">${escapeHtml(r.config.name)}</button></td><td>${iconButton('remove-folder', '解除目录关联', 'trash', `data-id="${escapeHtml(r.config.id)}" data-path="${escapeHtml(path)}"`)}</td></tr>`).join('')}</tbody></table><p class="page-note">仅显示已配置目录；自动发现与历史目录可在 Runner 中读取。</p>` : emptyState()}</div>`;
}
function activityPage() {
  if (!runnerById(app.logRunner)) app.logRunner = app.runners[0]?.config.id || '';
  return `${heading('日志', app.runners.length ? button('refresh-logs', '刷新', 'refresh') : '')}${app.runners.length ? `<div class="log-toolbar"><label for="log-runner">Runner</label><select id="log-runner">${app.runners.map((r) => `<option value="${escapeHtml(r.config.id)}" ${app.logRunner === r.config.id ? 'selected' : ''}>${escapeHtml(r.config.name)}</option>`).join('')}</select><small>最近 300 条 · 自动更新</small></div><div id="log-output" class="logs-panel" data-scroll="logs" aria-label="进程日志">正在读取…</div>` : emptyState()}`;
}
function settingsPage() {
  const s = app.startup;
  return `${heading('设置')}<div class="page-content settings-content" data-scroll="settings"><section class="settings-section"><h2>启动与恢复</h2><label class="toggle-row"><span class="toggle-info"><strong>登录 Windows 后启动 AMPM</strong><small>仅当前用户，静默进入托盘；保留当前数据目录和端口。</small></span><span class="switch"><input type="checkbox" id="login-startup" aria-label="登录 Windows 后启动 AMPM" ${s?.enabled ? 'checked' : ''} ${!s?.supported || app.busy ? 'disabled' : ''}><span></span></span></label>${s?.error ? `<p role="alert">${escapeHtml(s.error)}</p>` : ''}${!s?.supported ? '<p>登录自启仅支持 Windows 桌面模式。</p>' : ''}<p>在各 Runner 的“编辑”中选择随 AMPM 启动及异常重启次数。手动停止会取消待重启；AMPM 自身崩溃不自动恢复。</p></section>${cliSettings()}${updateSettings()}<section class="settings-section"><h2>本地服务</h2><dl class="properties"><dt>配置文件</dt><dd><code>${escapeHtml(app.config_path)}</code></dd><dt>内部地址</dt><dd><code>${escapeHtml(location.origin)}</code></dd><dt>关闭窗口</dt><dd>隐藏到托盘，Runner 继续运行</dd><dt>退出程序</dt><dd>托盘菜单退出，停止托管进程</dd></dl></section><details class="settings-section advanced"><summary>权限与兼容性</summary><p>Runner 继承当前用户权限，不是沙箱。内部服务仅监听回环地址，写操作使用会话令牌。</p><p>WSL 需独立安装并登录 Amp。强制结束管理器后，应检查 WSL 遗留进程。</p><p>Amp 自动更新可能替换进程；AMPM 不接管新进程。正常退出不会触发异常重启。需要稳定托管时，可在 Amp 设置中关闭：</p><pre>"amp.runner.autoUpdate.enabled": false</pre></details></div>`;
}
function cliBusy() {
  return app.cliPending || Object.values(app.cli).some((c) => ['checking', 'updating'].includes(c.status));
}
function cliSettings() {
  if (app.cliTarget && !runnerById(app.cliTarget)) app.cliTarget = '';
  const c = app.cli[app.cliTarget];
  const busy = cliBusy() || ['checking', 'downloading', 'restarting'].includes(app.update?.status);
  const running = app.runners.some(runnerBusy);
  const message = c?.error || ({ idle: '尚未检查更新', checking: '正在检查…', available: '有新版本可用', current: '已是最新稳定版', newer: '当前版本高于最新稳定版', updating: '正在更新，请勿退出…' }[c?.status] || '尚未检测');
  return `<section class="settings-section" id="cli-updates"><div class="section-head"><h2>Amp CLI</h2><div class="heading-actions">${button('check-cli', '检查更新', 'refresh', '', busy ? 'disabled' : '')}${button('update-cli', c?.status === 'updating' ? '更新中…' : '更新 Amp', 'arrow-up', 'primary', busy || running || c?.status !== 'available' ? 'disabled' : '')}</div></div><div class="field cli-target"><label for="cli-target">安装环境</label><select id="cli-target" ${busy ? 'disabled' : ''}><option value="">本机默认</option>${app.runners.map(({ config: r }) => `<option value="${escapeHtml(r.id)}" ${r.id === app.cliTarget ? 'selected' : ''}>${escapeHtml(r.name)} · ${r.backend === 'wsl' ? `WSL / ${escapeHtml(r.distribution || '默认发行版')}` : '本机'}</option>`).join('')}</select></div><dl class="properties"><dt>当前版本</dt><dd><code title="${escapeHtml(c?.version || '')}">${escapeHtml(c?.version?.split(/\s+/)[0] || (c?.error ? '不可用' : '—'))}</code></dd><dt>最新稳定版</dt><dd><code>${escapeHtml(c?.latest || '—')}</code></dd><dt>可执行文件</dt><dd><code>${escapeHtml(c?.executable || '—')}</code></dd></dl><p class="cli-status ${c?.error ? 'cli-error' : ''}" role="status">${escapeHtml(message)}</p>${running && c?.status === 'available' ? '<small>请先停止所有托管 Runner</small>' : ''}</section>`;
}
function updateSettings() {
  const u = app.update;
  if (!u) return '';
  const busy = cliBusy() || ['checking', 'downloading', 'restarting'].includes(u.status);
  const running = app.runners.some(runnerBusy);
  const message = !u.supported ? '本地构建或无界面模式，未启用应用更新。' : u.error || ({ idle: '尚未检查更新', checking: '正在检查…', current: '已是最新版本', available: `发现新版本 ${u.latest}`, downloading: '正在下载并校验…', restarting: '更新已安装，正在重启…' }[u.status] || '');
  return `<section class="settings-section" id="app-updates"><div class="section-head"><h2>AMPM <span class="muted">${escapeHtml(u.current)}</span></h2>${button('check-update', '检查更新', 'refresh', '', !u.supported || busy ? 'disabled' : '')}</div><p role="status">${escapeHtml(message)}</p>${u.checked_at ? `<small>上次检查：${escapeHtml(new Date(u.checked_at).toLocaleString('zh-CN'))}</small>` : ''}${u.latest ? `<div class="section-head">${button('install-update', '安装并重启', 'refresh', 'primary', busy || running ? 'disabled' : '')}${running ? '<small>请先停止所有 Runner</small>' : ''}</div>` : ''}<label class="toggle-row"><span class="toggle-info"><strong>自动更新</strong><small>每 6 小时检查；窗口收起且 Runner 全部停止时安装。</small></span><span class="switch"><input type="checkbox" id="auto-update" aria-label="自动更新 AMPM" ${u.auto_update ? 'checked' : ''} ${!u.supported || busy ? 'disabled' : ''}><span></span></span></label></section>`;
}
function render() {
  const previous = document.activeElement;
  const cursor = previous?.id === 'search' ? previous.selectionStart : null;
  const scroll = Object.fromEntries($$('[data-scroll]').map((el) => [el.dataset.scroll, el.scrollTop]));
  const page = location.hash.slice(1);
  app.page = names[page] ? page : 'runners';
  document.title = `AMPM · ${names[app.page]}`;
  $$('[data-nav]').forEach((el) => { el.classList.toggle('active', el.dataset.nav === app.page); if (el.dataset.nav === app.page) el.setAttribute('aria-current', 'page'); else el.removeAttribute('aria-current'); });
  $('#main').innerHTML = ({ runners: runnersPage, folders: foldersPage, activity: activityPage, settings: settingsPage })[app.page]();
  if (app.page === 'runners') renderRunnerList();
  if (app.page === 'activity') refreshLogs().catch((error) => toast(error.message, true));
  $$('[data-scroll]').forEach((el) => { el.scrollTop = scroll[el.dataset.scroll] || 0; });
  const match = previous?.id ? document.getElementById(previous.id) : $$('[data-action], [data-select], [data-filter]').find((el) => previous?.dataset.select ? el.dataset.select === previous.dataset.select : previous?.dataset.filter ? el.dataset.filter === previous.dataset.filter : previous?.dataset.action && el.dataset.action === previous.dataset.action && el.dataset.id === previous.dataset.id && el.dataset.path === previous.dataset.path);
  if (match && !match.closest('dialog')) match.focus({ preventScroll: true });
  if (cursor !== null && $('#search')) { try { $('#search').setSelectionRange(cursor, cursor); } catch { /* Search inputs vary by browser. */ } }
}
function dialogHead(id, title, subtitle = '') {
  return `<div class="dialog-head"><div><h2 id="${id}-title">${escapeHtml(title)}</h2>${subtitle ? `<p>${escapeHtml(subtitle)}</p>` : ''}</div>${iconButton('close', '关闭', 'x')}</div>`;
}
function inputField(name, label, value, placeholder, extra = '') {
  return `<div class="field"><label for="field-${name}">${label}</label><input id="field-${name}" name="${name}" value="${escapeHtml(value)}" placeholder="${escapeHtml(placeholder)}" ${extra}></div>`;
}
function openEditor(id) {
  const existing = id ? runnerById(id) : null;
  if (existing && runnerBusy(existing)) { toast('请先停止 Runner（包括待重启）再编辑启动配置。', true); return; }
  const c = existing?.config || { id: '', name: '', directories: [], backend: 'native', executable: 'amp', distribution: '', discover: false, terminal: false, share: false };
  const dialog = $('#editor');
  dialog.innerHTML = `<form id="runner-form">${dialogHead('editor', existing ? '编辑 Runner' : '新建 Runner')}<div class="dialog-body"><div class="form-grid">${inputField('name', '显示名称', c.name, '例如：日常开发', 'required maxlength="80"')}${inputField('id', 'Runner ID', c.id, '例如：dev-workstation', 'required maxlength="63" pattern="[a-zA-Z0-9]([a-zA-Z0-9\\-]*[a-zA-Z0-9])?"')}</div><div class="section-label">运行环境</div><div class="form-grid"><div class="field"><label for="field-backend">启动方式</label><select name="backend" id="field-backend"><option value="native" ${c.backend === 'native' ? 'selected' : ''}>本机原生 · ${escapeHtml(app.platform)}</option>${app.platform === 'windows' ? `<option value="wsl" ${c.backend === 'wsl' ? 'selected' : ''}>WSL · Linux 子系统</option>` : ''}</select></div>${inputField('executable', 'Amp 可执行文件', c.executable, 'amp 或绝对路径', 'required')}</div><div class="field" id="distribution-field" ${c.backend === 'wsl' ? '' : 'hidden'}><label for="field-distribution">WSL 发行版 <span class="optional">可选，留空使用默认发行版</span></label><input name="distribution" id="field-distribution" value="${escapeHtml(c.distribution)}" placeholder="Ubuntu"><small>WSL 需要 setsid、sh、kill，及已登录的 Linux 版 Amp。</small></div><div class="section-label">目录</div><div class="field"><div class="input-row"><textarea name="directories" id="field-directories" aria-label="目录" required rows="4" placeholder="每行一个绝对路径">${escapeHtml(c.directories.join('\n'))}</textarea>${button('browse', '添加', 'plus', '', 'data-target="field-directories"')}</div></div><div class="section-label">特性</div>${[['discover', '自动发现项目', '每分钟扫描列表第一项下两级 Git 项目，最多 200 个。', 'scan'], ['terminal', '远程终端', '允许从 Amp 网页访问本机终端。', 'terminal'], ['share', '共享给工作区', '工作区成员可在 ampcode.com 的 Shared Runners 中使用此 Runner 创建线程。', 'users']].map(([name, label, description, symbol]) => `<label class="toggle-row"><span class="toggle-icon">${icon(symbol)}</span><span class="toggle-info"><strong>${label}</strong><small>${description}</small></span><span class="switch"><input type="checkbox" name="${name}" aria-label="${label}" ${c[name] ? 'checked' : ''}><span></span></span></label>`).join('')}<div id="terminal-warning" class="notice" ${c.terminal ? '' : 'hidden'}>远程终端具有当前用户权限，可访问文件、凭据与网络。</div><div id="share-warning" class="notice" ${c.share ? '' : 'hidden'}>工作区所有成员都将以你的身份在本机运行代码，可使用你的文件、凭据与登录状态，并可能同时修改相同目录。仅共享给你愿意交出本机 Shell 的人。需要 2026-09-24 之后发布的 Amp CLI，旧版可在“设置 → Amp CLI”更新。</div><div class="form-error" role="alert" hidden></div></div><div class="dialog-footer">${existing ? button('delete', '删除配置', 'trash', 'danger subtle', `data-id="${escapeHtml(id)}"`) : ''}${button('close', '取消', '')}<button type="submit" class="btn primary">${icon('check')}${existing ? '保存更改' : '创建 Runner'}</button></div></form>`;
  $('.form-error', dialog).insertAdjacentHTML('beforebegin', `<div class="section-label">启动与恢复</div><label class="toggle-row"><span class="toggle-info"><strong>随 AMPM 启动</strong><small>每次打开 AMPM 自动启动此 Runner，使用已配置的目录、终端及共享权限。</small></span><span class="switch"><input type="checkbox" name="autostart" aria-label="随 AMPM 启动" ${c.autostart ? 'checked' : ''}><span></span></span></label>${inputField('restart_limit', '异常退出自动重启次数', c.restart_limit || 0, '0 表示关闭', 'type="number" min="0" max="20" step="1" required')}<p class="env-note">0 表示关闭，最多 20 次，每次间隔 5 秒。仅非零退出或启动失败重试；正常退出不重启。手动停止取消重试，手动启动或 AMPM 重新启动后计数重置。</p>`);
  updateBackend();
  $('#runner-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    if (app.busy) return;
    const data = new FormData(event.target);
    const body = Object.fromEntries(['id', 'name', 'executable', 'backend', 'distribution'].map((key) => [key, String(data.get(key) || '').trim()]));
    body.directories = String(data.get('directories')).split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
    body.discover = data.has('discover'); body.terminal = data.has('terminal'); body.share = data.has('share');
    body.autostart = data.has('autostart'); body.restart_limit = Number(data.get('restart_limit'));
    app.busy = true;
    const submit = $('button[type=submit]', dialog); submit.disabled = true;
    try { await api(existing ? runnerUrl(id) : '/api/runners', body, existing ? 'PUT' : 'POST'); dialog.close(); app.focused = body.id; toast(existing ? '配置已保存，下次启动时生效' : 'Runner 已创建，尚未启动'); await refresh(true); }
    catch (error) { formError(dialog, error); }
    finally { app.busy = false; submit.disabled = false; render(); }
  });
  dialog.showModal();
}
function updateBackend() {
  const isWsl = $('#field-backend')?.value === 'wsl';
  if ($('#distribution-field')) $('#distribution-field').hidden = !isWsl;
  $('#field-directories').placeholder = isWsl || app.platform !== 'windows' ? '/home/me/projects\n/home/me/another-project' : 'D:\\Projects\nE:\\AnotherProject';
  $$('#runner-form [data-action=browse]').forEach((el) => { el.disabled = isWsl; el.title = isWsl ? 'WSL 目录请填写 Linux 绝对路径' : '浏览本机文件夹'; });
}
let browseTarget = null, browseCurrent = '';
async function openBrowser(target) {
  browseTarget = target;
  const dialog = $('#browser');
  dialog.innerHTML = `${dialogHead('browser', '选择文件夹')}<form class="browse-path" id="browse-form">${button('browse-up', '', 'arrow-up', '', 'aria-label="上一级"')}<input class="path-input mono" id="browse-location" aria-label="目录路径" placeholder="输入绝对路径"><button class="btn" type="submit">前往</button></form><div class="browse-list" id="browse-list">正在读取…</div><div class="dialog-footer">${button('close', '取消', '')}${button('choose-folder', '选择此文件夹', 'check', 'primary', 'disabled')}</div>`;
  $('#browse-form').addEventListener('submit', (event) => { event.preventDefault(); loadBrowser($('#browse-location').value).catch((e) => toast(e.message, true)); });
  dialog.showModal();
  await loadBrowser(target?.tagName === 'INPUT' ? target.value : '');
}
async function loadBrowser(path) {
  const result = await api(`/api/browse?path=${encodeURIComponent(path)}`);
  browseCurrent = result.path;
  $('#browse-location').value = result.path;
  $('[data-action=browse-up]', $('#browser')).dataset.path = result.parent ?? '';
  $('[data-action=choose-folder]', $('#browser')).disabled = !result.path;
  $('#browse-list').innerHTML = result.entries.length ? result.entries.map((entry) => `<button class="browse-entry" data-action="browse-enter" data-path="${escapeHtml(entry.path)}">${icon('folder')}${escapeHtml(entry.name)}${icon('chevron-right')}</button>`).join('') : '<p class="muted">没有子文件夹。可以直接选择当前目录。</p>';
}
function openFolder(id) {
  if (!app.runners.length) { openEditor(); return; }
  const dialog = $('#editor');
  dialog.innerHTML = `<form id="folder-form">${dialogHead('editor', '添加目录')}<div class="dialog-body"><div class="field"><label for="folder-runner">所属 Runner</label><select name="runner" id="folder-runner">${app.runners.map((r) => `<option value="${escapeHtml(r.config.id)}" ${r.config.id === id ? 'selected' : ''}>${escapeHtml(r.config.name)} · ${r.config.backend}</option>`).join('')}</select></div><div class="field"><label for="folder-paths">文件夹绝对路径 <span class="optional">每行一个</span></label><div class="input-row"><textarea id="folder-paths" name="paths" required placeholder="填写需要共享的目录"></textarea>${button('browse', '浏览', 'folder', '', 'data-target="folder-paths"')}</div></div><div class="notice">运行中立即添加；已停止则下次启动生效。Amp 可能记忆这些目录。</div><div class="form-error" role="alert" hidden></div></div><div class="dialog-footer">${button('close', '取消', '')}<button type="submit" class="btn primary">${icon('plus')}添加目录</button></div></form>`;
  const setBrowse = () => { $('[data-action=browse]', dialog).disabled = runnerById($('#folder-runner').value)?.config.backend === 'wsl'; };
  $('#folder-runner').addEventListener('change', setBrowse); setBrowse();
  $('#folder-form').addEventListener('submit', async (event) => {
    event.preventDefault(); if (app.busy) return;
    const paths = [...new Set($('#folder-paths').value.split(/\r?\n/).map((s) => s.trim()).filter(Boolean))];
    if (!paths.length) return;
    if (paths.length > 200) { formError(dialog, new Error('每次最多添加 200 个目录')); return; }
    const runner = $('#folder-runner').value;
    app.busy = true; $('button[type=submit]', dialog).disabled = true;
    const errors = [], failed = [];
    for (const path of paths) {
      try { await api(`${runnerUrl(runner)}/directories`, { action: 'add', path }); }
      catch (error) { errors.push(`${path}：${error.message}`); failed.push(path); }
    }
    app.busy = false; $('button[type=submit]', dialog).disabled = false;
    await refresh(true);
    if (errors.length) { $('#folder-paths').value = failed.join('\n'); formError(dialog, new Error(`已添加 ${paths.length - failed.length} 个，${failed.length} 个失败：\n${errors.join('\n')}`)); }
    else { dialog.close(); toast(`已添加 ${paths.length} 个目录`); }
  });
  dialog.showModal();
}
function openDetails(id) {
  if (!runnerById(id)) return;
  app.focused = id;
  if (!visibleRunners().some((r) => r.config.id === id)) { app.filter = 'all'; app.search = ''; }
  if (app.page !== 'runners') location.hash = 'runners';
  else { renderRunnerList(); $('#runner-detail').scrollTop = 0; }
}
async function refreshLogs() {
  const id = app.logRunner;
  if (!id || !$('#log-output')) return;
  const result = await api(`${runnerUrl(id)}/logs`);
  if (!$('#log-output') || app.logRunner !== id) return;
  const output = $('#log-output');
  const atBottom = output.scrollHeight - output.scrollTop - output.clientHeight < 40;
  output.innerHTML = result.lines.length ? result.lines.map((line) => `<div class="log-${escapeHtml(line.stream)}"><span class="log-time">${new Date(line.time).toLocaleTimeString('zh-CN', { hour12: false })}</span>  [${escapeHtml(line.stream)}] ${escapeHtml(line.text)}</div>`).join('') : '<span class="log-time">暂无日志</span>';
  if (atBottom) output.scrollTop = output.scrollHeight;
}
async function checkCli(latest = false) {
  if (cliBusy()) return;
  const runner_id = app.cliTarget;
  app.cliPending = true;
  if (app.loaded) render();
  try {
    app.cli[runner_id] = latest ? await api('/api/cli/check', { runner_id }) : await api(`/api/cli?runner_id=${encodeURIComponent(runner_id)}`);
  } finally {
    app.cliPending = false;
    await refresh(true);
  }
}
async function updateCli() {
  if (cliBusy()) return;
  const runner_id = app.cliTarget, c = app.cli[runner_id];
  if (c?.status !== 'available') return;
  if (!await confirmAction('更新 Amp CLI？', `${c.executable}\n将使用此环境的 amp update 安装最新稳定版。请先结束其他 Amp 会话；不会自动停止或重启 Runner。`, '更新 Amp')) return;
  app.cliPending = true; render();
  try { await api('/api/cli/update', { runner_id }); toast('Amp 版本已确认更新'); }
  finally { app.cliPending = false; await refresh(true); }
}
async function runAction(id, action) {
  if (app.busy) return;
  const r = runnerById(id); if (!r) return;
  if (action !== 'start' && !await confirmAction(action === 'stop' ? '停止 Runner？' : '重启 Runner？', `${r.config.name} 上进行中的任务可能中断。只操作由本管理器托管的进程。`, action === 'stop' ? '停止' : '重启')) return;
  if (action === 'start' && !await confirmAction('启动本地 Runner？', `将启动 ${r.config.name}，向已登录的 Amp 账号提供配置的目录${r.config.terminal ? '，并开放远程终端权限' : '（远程终端关闭）'}${r.config.share ? '；已共享给整个工作区，成员将以你的身份在本机运行代码' : ''}。Runner 会继承当前系统用户权限。`, '启动 Runner')) return;
  app.busy = true; render();
  try { await api(`${runnerUrl(id)}/action`, { action }); toast(action === 'start' ? '进程已启动，请在日志中确认 Amp 连接' : action === 'stop' ? 'Runner 已停止' : 'Runner 已重新启动'); }
  finally { app.busy = false; await refresh(true); }
}
async function batchAction(action) {
  if (app.busy || !app.selected.size) return;
  const ids = [...app.selected];
  const verb = { start: '启动', stop: '停止', restart: '重启' }[action];
  if (!await confirmAction(`批量${verb} ${ids.length} 个 Runner？`, action === 'start' ? '将按各自配置提供目录和终端权限。已运行的 Runner 会返回错误，不重复启动。' : '这些 Runner 上进行中的任务可能中断。每个操作会独立返回结果。', `确认${verb}`)) return;
  app.busy = true; render();
  try {
    const result = await api('/api/batch', { ids, action });
    const failed = result.results.filter((r) => !r.ok);
    toast(`${result.results.length - failed.length} 个操作成功${failed.length ? `，${failed.length} 个失败：\n${failed.map((r) => `${r.id}：${r.error}`).join('\n')}` : ''}`, failed.length > 0);
  } finally { app.busy = false; await refresh(true); }
}

document.addEventListener('click', async (event) => {
  const filter = event.target.closest('[data-filter]');
  if (filter) { app.filter = filter.dataset.filter; render(); return; }
  const el = event.target.closest('[data-action]'); if (!el || el.disabled) return;
  const { action, id, path } = el.dataset;
  try {
    if (['start', 'stop', 'restart'].includes(action)) return await runAction(id, action);
    if (action.startsWith('batch-')) return await batchAction(action.slice(6));
    switch (action) {
      case 'new': openEditor(); break;
      case 'edit': $('#details').close(); openEditor(id); break;
      case 'details': openDetails(id); break;
      case 'close': el.closest('dialog').close(); break;
      case 'refresh': await refresh(true); break;
      case 'clear-selection': app.selected.clear(); renderRunnerList(); break;
      case 'clear-filter': app.filter = 'all'; app.search = ''; render(); break;
      case 'add-folder': $('#details').close(); openFolder(id); break;
      case 'browse': await openBrowser(document.getElementById(el.dataset.target)); break;
      case 'browse-enter': case 'browse-up': await loadBrowser(path); break;
      case 'choose-folder':
        if (browseTarget) { browseTarget.value = browseTarget.tagName === 'TEXTAREA' ? [browseTarget.value.trim(), browseCurrent].filter(Boolean).join('\n') : browseCurrent; browseTarget.dispatchEvent(new Event('input', { bubbles: true })); }
        $('#browser').close(); break;
      case 'logs': $('#details').close(); app.logRunner = id; location.hash = 'activity'; if (app.page === 'activity') render(); break;
      case 'refresh-logs': await refreshLogs(); break;
      case 'check-cli': await checkCli(true); break;
      case 'update-cli': await updateCli(); break;
      case 'check-update':
        el.disabled = true;
        try { await api('/api/update/check', {}); }
        finally { await refresh(true); }
        break;
      case 'install-update':
        if (!await confirmAction('安装更新？', '更新完成后 AMPM 将重启。配置和目录保持不变；运行中的 Runner 不会被强制停止。', '安装并重启')) break;
        el.disabled = true;
        try { await api('/api/update/install', {}); toast('新版已安装，正在重启'); }
        finally { await refresh(true); }
        break;
      case 'delete':
        if (app.busy || !await confirmAction('删除 Runner 配置？', '仅从 AMPM 移除此记录和本次日志，不删除磁盘文件，不修改 Amp 的历史目录记录。', '删除配置')) break;
        await api(runnerUrl(id), {}, 'DELETE'); $('#editor').close(); app.selected.delete(id); toast('配置已删除'); await refresh(true); break;
      case 'remove-folder':
        if (app.busy || !await confirmAction('解除目录关联？', `${path}\n\n不删除磁盘文件。启动时提供的目录必须先停止才能移除；Amp 记忆的目录可能仍需通过 CLI 移除。`, '解除关联')) break;
        await api(`${runnerUrl(id)}/directories`, { action: 'remove', path }); await refresh(true); toast('目录关联已移除'); break;
      case 'inspect': {
        if (app.inspect?.loading) break;
        app.inspect = { id, text: '正在读取…', loading: true };
        renderRunnerList();
        try { app.inspect.text = (await api(`${runnerUrl(id)}/inspect`, {})).output; }
        catch (error) { app.inspect.text = error.message; }
        finally { app.inspect.loading = false; renderRunnerList(); }
        break;
      }
      case 'external': {
        const dialog = $('#details');
        dialog.innerHTML = `${dialogHead('details', '本机 Runner 发现', '原生环境 · 只读检测，不接管外部进程')}<div class="dialog-body"><p class="env-note">执行 amp runner list，以下是当前 CLI 的原始输出。</p><pre class="terminal-output" id="external-output">正在检测…</pre></div><div class="dialog-footer">${button('close', '完成', '', 'primary')}</div>`;
        dialog.showModal();
        try { const result = await api('/api/external'); $('#external-output').textContent = `${result.output}\n${result.note}`; }
        catch (error) { if ($('#external-output')) $('#external-output').textContent = error.message; }
        break;
      }
    }
  } catch (error) { toast(error.message, true); }
});
document.addEventListener('input', (event) => { if (event.target.id === 'search') { app.search = event.target.value; renderRunnerList(); } });
document.addEventListener('change', async (event) => {
  const el = event.target;
  if (el.dataset.select) { if (el.checked) app.selected.add(el.dataset.select); else app.selected.delete(el.dataset.select); renderRunnerList(); }
  if (el.id === 'select-all') { visibleRunners().forEach((r) => { if (el.checked) app.selected.add(r.config.id); else app.selected.delete(r.config.id); }); renderRunnerList(); }
  if (el.id === 'field-backend') updateBackend();
  if (el.name === 'terminal' && $('#terminal-warning')) $('#terminal-warning').hidden = !el.checked;
  if (el.name === 'share' && $('#share-warning')) $('#share-warning').hidden = !el.checked;
  if (el.id === 'log-runner') { app.logRunner = el.value; refreshLogs().catch((error) => toast(error.message, true)); }
  if (el.id === 'cli-target') {
    app.cliTarget = el.value;
    render();
    if (!app.cli[app.cliTarget]) checkCli().catch((error) => toast(error.message, true));
  }
  if (el.id === 'auto-update') {
    el.disabled = true;
    api('/api/update/preferences', { auto_update: el.checked }).catch((error) => toast(error.message, true)).finally(() => refresh(true));
  }
  if (el.id === 'login-startup') {
    const enabled = el.checked;
    app.busy = true; el.disabled = true;
    try {
      if (enabled && !await confirmAction('启用登录自启？', '当前用户登录 Windows 后会自动启动此处的 AMPM，并启动已勾选“随 AMPM 启动”的 Runner。已有 AMPM 自启入口将改为当前程序、数据目录和端口。', '启用自启')) return;
      await api('/api/startup', { enabled }); toast(enabled ? '已启用登录自启' : '已关闭登录自启');
    }
    catch (error) { toast(error.message, true); }
    finally { app.busy = false; await refresh(true); }
  }
});
document.addEventListener('keydown', (event) => {
  if ($('dialog[open]') || /INPUT|TEXTAREA|SELECT/.test(event.target.tagName) || event.ctrlKey || event.metaKey || event.altKey) return;
  if (event.key === '/') { event.preventDefault(); if (app.page !== 'runners') { location.hash = 'runners'; setTimeout(() => $('#search')?.focus(), 0); } else $('#search')?.focus(); }
  if (event.key.toLowerCase() === 'n') { event.preventDefault(); openEditor(); }
});
window.addEventListener('hashchange', () => { if (app.loaded) render(); });
// Closing the native window preserves the editor. Do not auto-restart over that draft.
const reportEditor = () => window.ipc?.postMessage($('#editor').open ? 'editor-open' : 'editor-closed');
new MutationObserver(reportEditor).observe($('#editor'), { attributes: true, attributeFilter: ['open'] });
reportEditor();
async function poll() {
  if (!app.busy && !document.hidden) { await refresh(); if (app.page === 'activity') { try { await refreshLogs(); } catch { /* Connection indicator reports service outages. */ } } }
  setTimeout(poll, 3000);
}
refresh(true).then(() => { checkCli().catch((error) => toast(error.message, true)); setTimeout(poll, 3000); });
