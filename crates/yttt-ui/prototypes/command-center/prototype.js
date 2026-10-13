/* THROWAWAY PROTOTYPE — all records and operations below are simulated.
 * Three structurally different views: A project matrix, B decision inbox,
 * C execution hierarchy. No network calls, credentials, storage or real agents.
 */
'use strict';

const variants = {
  A: { name: '项目矩阵', title: '所有项目，一眼掌握', description: '固定项目位置，集中看见进展、阻塞和需要你的决定。' },
  B: { name: '待处理中心', title: '把注意力留给决定', description: '问题带着上下文来到你面前，不必逐个终端寻找。' },
  C: { name: '执行关系', title: '从目标，一路看到执行', description: '展开主 Agent 与派生执行，区分业务阻塞和观测异常。' },
};
const statusNames = { working: '执行中', waiting: '等你处理', external: '等外部依赖', failed: '执行失败', completed: '待验收', accepted: '已验收', unknown: '观测未知', idle: '未启动' };
const providerNames = { OMP: 'Oh My Pi', Claude: 'Claude Code', Codex: 'Codex' };
const e = value => String(value ?? '').replace(/[&<>"']/g, character => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[character]));
const badge = (status, label = statusNames[status]) => `<span class="badge ${e(status)}"><span class="status-dot"></span>${e(label)}</span>`;
const projectSeeds = [
  { id: 'yttt', name: 'yttt', goal: '多项目指挥台 · 观测与交互', host: 'mac-local', branch: 'feat/command-center', provider: 'OMP', model: 'GPT-6', status: 'working', stages: ['定义观测', '交互原型', '原生实现', '验证'], stage: 1, done: 2, total: 6, action: '实现项目总览与问题卡片', age: '12m', progress: '2m 前', context: '让多个项目同时推进时，人的注意力优先落在决策和阻塞上，而不是巡视终端。', acceptance: '目标归属清晰；回答路由准确；未知状态不误报完成。', children: [{ id: 'layout', name: '布局探索', provider: 'Claude', task: '对比三种大屏信息层级', status: 'working', age: '8m' }, { id: 'events', name: '事件梳理', provider: 'OMP', task: '整理任务与提问观测点', status: 'completed', age: '已结束' }] },
  { id: 'atlas', name: 'Atlas', goal: '账户体系 · 新旧接口迁移', host: 'mac-local', branch: 'feat/account-v2', provider: 'Claude', model: 'Sonnet', status: 'waiting', stages: ['方案', '接口迁移', '回归', '交付'], stage: 1, done: 3, total: 8, action: '等待决定旧客户端兼容策略', age: '4m', progress: '8m 前', context: '新接口已经实现。旧版客户端仍使用 v1，兼容策略将影响路由与回归范围。', acceptance: '新客户端可登录；旧版兼容范围有明确决定；迁移可回滚。', children: [{ id: 'api', name: 'API 实现', provider: 'Codex', task: '等待主 Agent 确认接口策略', status: 'external', age: '4m' }, { id: 'tests', name: '回归验证', provider: 'OMP', task: '完成新接口集成用例', status: 'completed', age: '已结束' }] },
  { id: 'gateway', name: 'Gateway', goal: '网关鉴权 · 灰度验证', host: 'build-linux', branch: 'fix/auth-routing', provider: 'OMP', model: 'GPT-6', status: 'waiting', stages: ['复现', '修复', '灰度验证', '发布'], stage: 2, done: 5, total: 7, action: '等待授权读取预发布日志', age: '2m', progress: '5m 前', context: '本地用例已通过，需要读取预发布访问日志验证灰度请求。此次操作只读，不执行部署。', acceptance: '鉴权失败率恢复；灰度日志无回归；生产发布另行授权。', children: [{ id: 'trace', name: '请求追踪', provider: 'Claude', task: '准备核对灰度请求链路', status: 'external', age: '2m' }] },
  { id: 'studio', name: 'Studio', goal: '编辑器 · 大文件滚动优化', host: 'mac-local', branch: 'perf/large-document', provider: 'Codex', model: 'GPT-6', status: 'working', stages: ['分析', '优化', '基准验证', '交付'], stage: 2, done: 4, total: 6, action: '执行滚动延迟基准', age: '23m', progress: '1m 前', context: '减少超长文档滚动时的重复布局，保留选择和光标行为。', acceptance: '大文件滚动无明显停顿；文本选择和编辑行为不变。', children: [{ id: 'benchmark', name: '性能验证', provider: 'OMP', task: '对比相同文档滚动延迟', status: 'working', age: '6m' }] },
  { id: 'mobile', name: 'Mobile', goal: '移动端 · 会话恢复', host: 'mac-local', branch: 'feat/session-restore', provider: 'Claude', model: 'Sonnet', status: 'waiting', stages: ['设计', '实现', '设备验证', '交付'], stage: 2, done: 4, total: 7, action: '等待确认断线后的恢复行为', age: '1m', progress: '3m 前', context: '用户可能在断连后编辑输入。恢复时不能把未发送草稿自动提交给 Agent。', acceptance: '草稿不丢失；重连不重复执行；只恢复确认过的会话。', children: [] },
  { id: 'release', name: 'Release', goal: 'Windows · 安装包构建', host: 'win-runner', branch: 'release/0.3.9', provider: 'OMP', model: 'GPT-6', status: 'failed', stages: ['打包', '构建', '校验', '发布'], stage: 1, done: 2, total: 5, action: '签名步骤失败 · 凭据不可用', age: '7m', progress: '7m 前', context: '安装包构建到签名步骤失败。源码构建已完成，未上传或发布任何安装包。', acceptance: '安装与卸载可用；校验和匹配；签名状态明确。', children: [{ id: 'pack', name: '安装包打包', provider: 'Codex', task: '生成未签名构建产物', status: 'completed', age: '已结束' }, { id: 'sign', name: '签名验证', provider: 'OMP', task: '签名凭据不可用', status: 'failed', age: '7m' }] },
  { id: 'docs', name: 'Docs', goal: '文档 · 远端接入指南', host: 'mac-local', branch: 'docs/remote-guide', provider: 'Claude', model: 'Sonnet', status: 'completed', stages: ['整理', '编写', '校对', '验收'], stage: 3, done: 5, total: 6, action: '交付已就绪 · 等待人工验收', age: '18m', progress: '6m 前', context: '远端接入、指纹确认和断线恢复的文档已整理，等待人工核对步骤。', acceptance: '按文档可完成首次接入；风险提示完整；链接可访问。', children: [] },
  { id: 'pipeline', name: 'Pipeline', goal: 'CI · 发布资产校验', host: 'build-linux', branch: 'fix/release-checks', provider: 'OMP', model: 'GPT-6', status: 'external', stages: ['准备', '构建', '资产校验', '交付'], stage: 2, done: 3, total: 5, action: '等待远端构建产物，不需要回答', age: '11m', progress: '11m 前', context: '校验任务已准备，需要等待构建产物出现后再核对下载地址和 SHA-256。', acceptance: '资产完整；每个目标平台均有校验和。', children: [{ id: 'assets', name: '资产检查', provider: 'Codex', task: '等待 CI 上传产物', status: 'external', age: '11m' }] },
  { id: 'design', name: 'Design System', goal: '组件库 · 键盘可访问性', host: 'mac-local', branch: 'fix/keyboard-focus', provider: 'Codex', model: 'GPT-6', status: 'working', stages: ['审计', '修复', '交互验证', '交付'], stage: 1, done: 2, total: 7, action: '修复弹窗关闭后的焦点返回', age: '31m', progress: '4m 前', context: '统一对话框、菜单和表单的键盘交互，保留可见焦点。', acceptance: 'Tab 顺序可预测；Escape 正确退出；焦点返回触发元素。', children: [{ id: 'audit', name: '键盘审计', provider: 'Claude', task: '检查菜单的焦点顺序', status: 'working', age: '9m' }] },
];
const questionSeeds = [
  { id: 'q-compat', project: 'atlas', kind: 'decision', title: '旧版客户端，是否保留 v1 兼容？', context: '新接口已经就绪。继续保留兼容层，旧客户端无需立即升级；直接切换则需要同步发布客户端。', impact: '阻塞 API 实现；改变路由与回归测试范围。', options: ['保留一个版本', '仅支持新接口'], age: '4m', phase: '接口迁移', source: '主 Agent · Claude', request: 'decision/atlas/017' },
  { id: 'q-logs', project: 'gateway', kind: 'approval', title: '允许读取预发布环境的访问日志？', context: '仅查询最近 15 分钟的鉴权日志，不写入文件、不修改配置、不执行部署。授权范围限定为这一次查询。', impact: '阻塞灰度验证；执行位置 build-linux。', options: ['允许这一次', '拒绝'], age: '2m', phase: '灰度验证', source: '主 Agent · OMP', request: 'approval/gateway/024' },
  { id: 'q-restore', project: 'mobile', kind: 'decision', title: '断线重连后，如何处理未发送的输入？', context: '本地草稿可能比 Host 上的会话更新。自动发送可能重复执行此前已经提交的操作。', impact: '决定恢复交互；不会改变已经运行的任务。', options: ['保留草稿，手动发送', '重连后提示确认'], age: '1m', phase: '设备验证', source: '主 Agent · Claude', request: 'decision/mobile/009' },
];
const additionalQuestions = [
  { id: 'q-perf', project: 'studio', kind: 'decision', title: '性能回归按哪个设备档位验收？', context: '当前样本来自开发机。低配设备与高刷新率设备的验收阈值不同，需要确认目标档位。', impact: '阻塞性能验收；不会自动扩大优化范围。', options: ['以最低支持设备为准', '同时覆盖两档设备'], age: '刚刚', phase: '基准验证', source: '主 Agent · Codex', request: 'decision/studio/031' },
  { id: 'q-focus', project: 'design', kind: 'decision', title: '焦点修复是否同步更新菜单交互？', context: '对话框的问题已定位，菜单存在相似但独立的键盘行为。需要确认是否属于这次交付。', impact: '改变任务范围；菜单可以单独交付。', options: ['只修复对话框', '菜单另建任务'], age: '刚刚', phase: '修复', source: '主 Agent · Codex', request: 'decision/design/012' },
];
let state;
let toastTimer;
let dialogTab = 'context';
const dialog = document.getElementById('detail-dialog');
function requestedVariant() { const value = new URLSearchParams(location.search).get('variant'); return variants[value] ? value : 'A'; }
function reset(scenario = 'normal', notify = false) {
  const variant = state?.variant || requestedVariant();
  state = { variant, scenario, selectedProject: scenario === 'offline' ? 'gateway' : 'atlas', selectedQuestion: 'q-compat', query: '', attentionOnly: false, inboxTab: 'pending', projects: structuredClone(projectSeeds).map((project, index) => ({ ...project, number: String(index + 1).padStart(2, '0'), run: 1, session: `demo/${project.id}/session-01`, accepted: false })), questions: structuredClone([...questionSeeds, ...(scenario === 'questions' ? additionalQuestions : [])]).map(question => ({ ...question, status: 'pending', answer: null, draft: '' })), disconnectedHosts: scenario === 'offline' ? ['build-linux'] : [], operations: [], events: [
    { time: '15:42:08', project: 'docs', text: '文档交付已就绪，等待人工验收', type: 'completed', source: 'Agent 上报 · 演示' },
    { time: '15:41:36', project: 'studio', text: '性能验证子 Agent 开始基准检查', type: 'working', source: '生命周期 · 演示' },
    { time: '15:40:12', project: 'release', text: '签名验证失败，产物未发布', type: 'failed', source: '执行结果 · 演示' },
  ] };
  for (const question of state.questions) { const project = getProject(question.project); project.status = 'waiting'; }
  if (scenario === 'offline') { state.selectedQuestion = 'q-logs'; addEvent('gateway', 'Host 连接中断；保留最后已知状态，未将任务标为失败', 'unknown'); }
  document.getElementById('search').value = '';
  document.getElementById('scenario').value = scenario;
  render();
  if (notify) toast('演示已重置。所有操作只影响当前页面，未连接真实 Agent。');
}
function getProject(id) { return state.projects.find(project => project.id === id); }
function offline(project) { return state.disconnectedHosts.includes(project.host); }
function viewStatus(project) { return offline(project) ? 'unknown' : project.accepted ? 'accepted' : project.status; }
function pendingQuestions(projectId) { return state.questions.filter(question => question.status === 'pending' && (!projectId || question.project === projectId)); }
function projectMatches(project) { return !state.query || `${project.name} ${project.goal} ${project.provider} ${project.host} ${project.children.map(child => `${child.name} ${child.provider}`).join(' ')}`.toLowerCase().includes(state.query.toLowerCase()); }
function visibleProjects() { return state.projects.filter(project => projectMatches(project) && (!state.attentionOnly || ['waiting', 'failed', 'unknown', 'completed'].includes(viewStatus(project)))); }
function visibleQuestions() { return pendingQuestions().filter(question => projectMatches(getProject(question.project))); }
function counts() {
  let running = 0; let descendants = 0;
  for (const project of state.projects) {
    if (!offline(project) && ['working', 'waiting', 'external'].includes(project.status)) running++;
    for (const child of project.children) { if (!offline(project) && ['working', 'waiting', 'external'].includes(child.status)) { running++; descendants++; } }
  }
  return { projects: state.projects.length, running, descendants, pending: pendingQuestions().length, issues: state.projects.filter(project => ['failed', 'unknown'].includes(viewStatus(project))).length, awaitingAcceptance: state.projects.filter(project => project.status === 'completed' && !project.accepted).length };
}
function renderMetrics() {
  const values = counts();
  document.getElementById('metrics').innerHTML = [
    ['项目', values.projects, '跨本地与 yttt 管理的远端', '3 HOSTS', ''],
    ['活跃 Agent', values.running, `含 ${values.descendants} 个派生执行 · 不含已结束`, 'ROOT + CHILD', 'blue'],
    ['需要你回答', values.pending, '决策与授权 · 不含外部等待', 'ACTION REQUIRED', 'amber'],
    ['异常 / 观测未知', values.issues, '执行失败与连接失联分别显示', 'ATTENTION', 'red'],
    ['等待验收', values.awaitingAcceptance, 'Agent 交付不等于人工验收', 'REVIEW', 'green'],
  ].map(([label, value, description, tag, color]) => `<div class="metric"><div><div class="metric-label">${label}</div><div class="metric-description">${description}</div></div><div><div class="metric-value ${color}">${value.toString().padStart(2, '0')}</div><div class="metric-tag">${tag}</div></div></div>`).join('');
}
function projectCard(project) {
  const status = viewStatus(project);
  return `<button class="project-card ${state.selectedProject === project.id ? 'selected' : ''}" data-action="project-detail" data-project="${project.id}" aria-label="查看 ${e(project.name)} 项目详情">
    <div class="card-top"><span class="project-name"><span class="project-number">${project.number}</span>${e(project.name)}</span>${badge(status, project.accepted ? '已验收' : statusNames[status])}</div>
    <div class="project-goal" title="${e(project.goal)}">${e(project.goal)}</div>
    <div class="stage-track" aria-hidden="true">${project.stages.map((_, index) => `<span class="stage-segment ${index < project.stage ? 'done' : index === project.stage ? `current ${status}` : ''}"></span>`).join('')}</div>
    <div class="stage-caption"><strong>${e(project.stages[project.stage])}</strong><span>已确认 ${project.done}/${project.total} 子任务</span></div>
    <div class="card-action"><span class="${status === 'waiting' ? 'amber' : status === 'failed' ? 'red' : 'blue'}">${status === 'waiting' ? '?' : status === 'failed' ? '!' : '›'}</span><span class="label">${e(offline(project) ? 'Host 失联 · 保留最后已知任务' : project.action)}</span></div>
    <div class="card-bottom"><span><span class="provider-mark">${e(project.provider)}</span> ${project.children.length ? `派生 ${project.children.length}` : '主会话'}</span><span>${e(project.host)} <span class="mono">· ${e(project.age)}</span></span></div>
  </button>`;
}
function questionCard(question, expanded = false) {
  const project = getProject(question.project); const blocked = offline(project);
  return `<article class="question-card" data-question="${question.id}">
    <div class="question-meta"><span><span class="question-project">${e(project.name)}</span> / ${e(question.phase)}</span><span class="mono">${e(question.age)} ${question.kind === 'approval' ? '· 授权' : '· 决策'}</span></div>
    <h3 class="question-title">${e(question.title)}</h3>
    ${expanded ? `<p class="question-context">${e(question.context)}</p>` : ''}
    <div class="question-effect">${e(question.impact)}</div>
    ${blocked ? '<div class="offline-note">Host 已失联。问题仍然保留，恢复连接前不能提交回答。</div>' : ''}
    <div class="question-options">${question.options.map((option, index) => `<button class="answer-button ${index === 0 ? 'recommended' : ''}" data-action="answer" data-question="${question.id}" data-option="${index}" ${blocked ? 'disabled' : ''}>${index === 0 ? '↳ ' : ''}${e(option)}</button>`).join('')}</div>
    <div class="question-foot"><span>${e(question.source)} · 演示</span><button data-action="question-detail" data-question="${question.id}">上下文 / 补充回答 ↗</button></div>
  </article>`;
}
function eventPanel() {
  return `<section class="panel activity-panel"><div class="panel-header"><div class="panel-heading"><h2>最近变化</h2><span class="panel-caption">保留已结束执行，不只显示当前状态</span></div><button class="text-button" data-action="history">查看全部 ↗</button></div><div class="activity-rows">${state.events.slice(0, 4).map(eventRow).join('')}</div></section>`;
}
function eventRow(event) { return `<div class="activity-row"><span class="activity-time">${e(event.time)}</span><span class="status-dot ${event.type === 'failed' ? 'red' : event.type === 'waiting' ? 'amber' : 'accent'}"></span><span class="activity-project">${e(getProject(event.project)?.name || event.project)}</span><span class="activity-text" title="${e(event.text)}">${e(event.text)}</span><span class="activity-origin">${e(event.source)}</span></div>`; }
function noQuestions(project) { return `<div class="empty"><span class="empty-mark">✓</span><strong>${project ? `${e(project.name)} 当前无需回答` : '当前没有待回答问题'}</strong><span>等外部依赖的任务不会进入这里。<br>${project ? '可以从上方队列选择其他项目的问题。' : '也可以切换“集中提问”场景继续体验。'}</span></div>`; }
function VariantA() {
  const projects = visibleProjects(); const questions = visibleQuestions();
  return `<div class="matrix-layout"><div class="matrix-main"><section class="project-section"><div class="section-header"><h2>项目总览 <b>${projects.length}</b></h2><button class="filter-button ${state.attentionOnly ? 'active' : ''}" data-action="attention" aria-pressed="${state.attentionOnly}">仅看需要介入</button></div><div class="project-grid">${projects.map(projectCard).join('') || '<div class="empty">没有匹配的项目</div>'}</div></section>${eventPanel()}</div><aside class="panel decision-rail"><div class="panel-header"><div class="panel-heading"><h2>需要你处理</h2><span class="count">${questions.length}</span></div><span class="panel-caption">等人，不是等工具</span></div><div class="decision-list scroll">${questions.map(question => questionCard(question)).join('') || noQuestions()}</div><div class="rail-bottom">点选回答后，查看数量与项目状态的联动。<br>所有回答均为本地模拟，不会发送给真实 Agent。</div></aside></div>`;
}
function projectIndex(projects) {
  return `<aside class="panel project-index"><div class="panel-header"><h2>项目索引</h2><span class="count">${projects.length}</span></div><div class="index-items scroll">${projects.map(project => `<button class="index-project ${state.selectedProject === project.id ? 'active' : ''}" data-action="select-project" data-project="${project.id}"><span class="index-avatar">${project.number}</span><span class="index-project-copy"><strong>${e(project.name)}</strong><small>${e(project.stages[project.stage])} · ${e(statusNames[viewStatus(project)])}</small></span>${pendingQuestions(project.id).length ? `<span class="index-count">${pendingQuestions(project.id).length}</span>` : ''}</button>`).join('') || '<div class="empty">没有匹配项目</div>'}</div><div class="rail-bottom">选择项目查看上下文。<br>中间队列保留所有待处理事项。</div></aside>`;
}
function steps(project) { return `<div class="step-list">${project.stages.map((name, index) => `<div class="step ${index < project.stage ? 'done' : index === project.stage ? 'current' : ''}"><span class="step-number">${index < project.stage ? '✓' : index + 1}</span><span>${e(name)}</span>${index === project.stage ? '<span class="muted">← 当前上报</span>' : ''}</div>`).join('')}</div>`; }
function agentList(project) {
  return [{ name: `主 Agent · ${project.provider}`, task: project.action, status: viewStatus(project) }, ...project.children.map(child => ({ ...child, status: offline(project) ? 'unknown' : child.status }))].map(agent => `<div class="mini-agent"><span class="status-dot ${agent.status === 'working' ? 'blue' : agent.status === 'waiting' ? 'amber' : agent.status === 'failed' ? 'red' : 'muted'}"></span><div class="mini-agent-copy"><strong>${e(agent.name)}</strong><small>${e(agent.task)}</small></div>${badge(agent.status, agent.status === 'completed' ? '已结束' : statusNames[agent.status])}</div>`).join('');
}
function contextPanel(project) {
  return `<aside class="panel context-panel"><div class="panel-header"><h2>${e(project.name)} · 任务上下文</h2><button class="text-button" data-action="project-detail" data-project="${project.id}">展开 ↗</button></div><div class="context-content scroll"><section class="context-section"><div class="context-label">当前目标 · 人工确认 / 演示</div><h3 class="context-goal">${e(project.goal)}</h3><p class="context-paragraph" style="margin-top:9px">${e(project.context)}</p></section><section class="context-section"><div class="context-label">阶段 · AGENT 上报 / 演示</div>${steps(project)}</section><section class="context-section"><div class="context-label">验收条件</div><p class="context-paragraph">${e(project.acceptance)}</p></section><section class="context-section"><div class="context-label">执行归属</div>${agentList(project)}</section><button class="context-link" data-action="terminal" data-project="${project.id}">查看演示执行现场 ↗</button></div></aside>`;
}
function replyEditor(question) { const blocked = offline(getProject(question.project)); return `<div class="reply-editor"><div class="reply-heading">或者，补充你的决定</div><textarea data-draft="${question.id}" aria-label="补充回答" placeholder="补充范围、约束或新的决定…" ${blocked ? 'disabled' : ''}>${e(question.draft)}</textarea><div class="reply-actions"><span>绑定 ${e(question.request)} · 仅模拟发送</span><button class="primary-button" data-action="custom-answer" data-question="${question.id}" ${blocked ? 'disabled' : ''}>提交回答</button></div></div>`; }
function VariantB() {
  const questions = visibleQuestions();
  const question = state.selectedQuestion === null ? null : questions.find(item => item.id === state.selectedQuestion) || questions.find(item => item.project === state.selectedProject) || questions[0];
  if (question && state.inboxTab === 'pending') { state.selectedQuestion = question.id; state.selectedProject = question.project; }
  const project = getProject(state.selectedProject);
  const resolved = state.questions.filter(item => item.status === 'answered' && projectMatches(getProject(item.project)));
  const stage = state.inboxTab === 'resolved' ? `<div class="panel decision-stage scroll"><div class="resolved-list">${resolved.map(item => `<article class="resolved-item"><span class="accent">${e(getProject(item.project).name)}</span><h3>${e(item.title)}</h3><p>你的回答：${e(item.answer)}</p><small>本地模拟已确认 · ${e(item.request)} · 记录仍然保留</small></article>`).join('') || '<div class="empty"><strong>还没有已处理的问题</strong><span>回答一个问题后，会保留在这里。</span></div>'}</div></div>` : `<div class="inbox-picker">${questions.map(item => `<button class="inbox-ticket ${item.id === question?.id ? 'active' : ''}" data-action="select-question" data-question="${item.id}"><strong>${e(getProject(item.project).name)} · ${item.kind === 'approval' ? '授权' : '决策'}</strong><small>${e(item.phase)} · 已等 ${e(item.age)}</small></button>`).join('')}</div><div class="panel decision-stage scroll">${question ? `${questionCard(question, true)}${replyEditor(question)}` : noQuestions(project)}</div>`;
  return `<div class="inbox-layout">${projectIndex(visibleProjects())}<section class="decision-workspace"><div class="inbox-tabs"><button class="inbox-tab ${state.inboxTab === 'pending' ? 'active' : ''}" data-action="inbox-tab" data-tab="pending">待处理 ${questions.length}</button><button class="inbox-tab ${state.inboxTab === 'resolved' ? 'active' : ''}" data-action="inbox-tab" data-tab="resolved">已处理 ${resolved.length}</button><span class="panel-caption" style="margin-left:auto;align-self:center">一次理解，一次决定</span></div>${stage}</section>${contextPanel(project)}</div>`;
}
function executionRow(project, child) {
  const status = offline(project) ? 'unknown' : child?.status || project.status;
  return `<div class="execution-row ${child ? 'child' : ''}"><div class="execution-agent">${child ? '<span class="tree-branch"></span>' : ''}<span class="provider-mark">${e(child?.provider || project.provider)}</span><span class="execution-agent-name">${e(child?.name || '主 Agent')}</span></div><span class="execution-task" title="${e(child?.task || project.action)}">${e(child?.task || project.action)}</span><span>${badge(status, child && status === 'completed' ? '已结束' : project.accepted && !child ? '已验收' : statusNames[status])}</span><span class="execution-age">${e(child?.age || project.age)}</span><button class="execution-action" data-action="execution-detail" data-project="${project.id}" ${child ? `data-child="${e(child.id)}"` : ''}>${child ? '执行记录' : '查看现场'} ↗</button></div>`;
}
function healthPanel(project) {
  const disconnected = offline(project);
  return `<aside class="panel context-panel"><div class="panel-header"><h2>${e(project.name)} · 运行观察</h2>${badge(viewStatus(project))}</div><div class="context-content scroll"><section class="context-section"><div class="context-label">当前任务</div><h3 class="context-goal">${e(project.goal)}</h3><p class="context-paragraph" style="margin-top:8px">${e(project.action)}</p></section><div class="context-label">三个信号，分别判断 · 演示</div><div class="health-line"><span>最后确认存活</span><span class="health-status ${disconnected ? 'red' : 'green'}">${disconnected ? '失联前 2m · 现未知' : '5s 前 · 存活'}</span></div><div class="health-line"><span>最后收到上报</span><span class="health-status">${disconnected ? '2m 前' : '12s 前'}</span></div><div class="health-line"><span>最近有效进展</span><span class="health-status">${e(project.progress)}</span></div><div class="signal"><div class="signal-label">${disconnected ? '观测中断 ≠ 执行失败' : project.status === 'external' ? '等待外部依赖 ≠ 等人回答' : project.status === 'failed' ? '失败范围' : '当前判断'}</div><p>${disconnected ? '无法确认远端当前状态。保留原任务和问题，连接恢复前禁用写操作。' : project.status === 'external' ? '任务正在等待构建产物，不进入人的待回答队列。' : project.status === 'failed' ? '签名步骤失败。已完成的打包结果仍保留，不将所有子 Agent 标记为失败。' : '进程、上报与任务进展是独立信号。这里不根据沉默时长自动判定失败。'}</p></div><dl class="data-pairs"><dt>执行位置</dt><dd>${e(project.host)}</dd><dt>工作分支</dt><dd class="mono">${e(project.branch)}</dd><dt>执行实例</dt><dd class="mono">${e(project.id)}/run-${project.run}</dd><dt>费用 / token</dt><dd>未接入，不显示为 0</dd><dt>控制范围</dt><dd>本页演示状态</dd></dl><div class="operation-buttons">${operationButtons(project)}</div><div class="subtle-rule"></div><button class="context-link" data-action="project-detail" data-project="${project.id}">目标、验收与证据 ↗</button><p class="context-paragraph" style="margin-top:14px">任何“恢复 / 回答 / 验收”都只改变本地演示数据。原型没有真实控制通路。</p></div></aside>`;
}
function operationButtons(project) {
  if (offline(project)) return `<button class="primary-button" data-action="reconnect" data-project="${project.id}">模拟恢复连接</button>`;
  if (project.status === 'failed') return `<button class="secondary-button" data-action="retry" data-project="${project.id}">模拟修复后重试</button>`;
  if (project.status === 'completed') return project.accepted ? '<span class="badge completed">人工验收已记录 · 演示</span>' : `<button class="primary-button" data-action="accept" data-project="${project.id}">模拟验收通过</button>`;
  const question = pendingQuestions(project.id)[0];
  if (question) return `<button class="secondary-button" data-action="question-detail" data-question="${question.id}">处理待回答问题</button>`;
  return `<button class="secondary-button" data-action="terminal" data-project="${project.id}">查看执行现场</button>`;
}
function VariantC() {
  const projects = visibleProjects(); const selected = getProject(state.selectedProject);
  return `<div class="topology-layout"><section class="panel execution-panel"><div class="panel-header"><div class="panel-heading"><h2>执行关系</h2><span class="count">${projects.reduce((sum, project) => sum + 1 + project.children.length, 0)} 条记录</span></div><button class="filter-button ${state.attentionOnly ? 'active' : ''}" data-action="attention" aria-pressed="${state.attentionOnly}">仅看需要介入</button></div>${state.disconnectedHosts.length ? `<div class="notice">${e(state.disconnectedHosts.join('、'))} 已失联。以下保留最后已知执行关系，当前状态标为未知。</div>` : ''}<div class="execution-columns"><span>项目 / Agent</span><span>正在推进什么</span><span>状态</span><span>持续时间</span><span>检查</span></div><div class="execution-body scroll">${projects.map(project => `<section class="execution-group ${state.selectedProject === project.id ? 'selected' : ''}"><div class="execution-project-heading"><strong><span class="project-number">${project.number}</span> ${e(project.name)} <span class="host-label"> / ${e(project.host)}</span></strong><button data-action="select-project" data-project="${project.id}">运行观察 →</button></div>${executionRow(project)}${project.children.map(child => executionRow(project, child)).join('')}</section>`).join('') || '<div class="empty">没有匹配的执行</div>'}</div><div class="legend"><span><i class="status-dot blue"></i>正在执行</span><span><i class="status-dot amber"></i>等待人</span><span><i class="status-dot muted"></i>未知不等于失败</span><span>已结束派生执行保留记录</span></div></section>${healthPanel(selected)}</div>`;
}
function PrototypeSwitcher() {
  return `<button class="switch-arrow" data-action="previous-variant" aria-label="上一个布局">←</button><span class="switch-caption">布局对比</span>${Object.entries(variants).map(([key, variant]) => `<button class="${state.variant === key ? 'active' : ''}" data-action="variant" data-variant="${key}" aria-current="${state.variant === key ? 'page' : 'false'}">${key} · ${variant.name}</button>`).join('')}<button class="switch-arrow" data-action="next-variant" aria-label="下一个布局">→</button>`;
}
function render() {
  document.getElementById('page-title').innerHTML = `${variants[state.variant].title}<span class="heading-dot"></span>`;
  document.getElementById('page-description').textContent = variants[state.variant].description;
  renderMetrics();
  document.getElementById('workspace').innerHTML = ({ A: VariantA, B: VariantB, C: VariantC })[state.variant]();
  document.getElementById('variant-switcher').innerHTML = PrototypeSwitcher();
  document.getElementById('state-summary').innerHTML = `<span class="status-dot"></span>本地演示 · ${state.scenario === 'normal' ? '多项目并行' : state.scenario === 'questions' ? '集中提问' : 'Host 失联场景'} · 已处理 ${state.questions.filter(question => question.status === 'answered').length} 项`;
  window.prototypeState = state;
}
function setVariant(variant) {
  if (!variants[variant]) return;
  state.variant = variant;
  const url = new URL(location.href); url.searchParams.set('variant', variant); history.replaceState({}, '', url);
  render();
  console.info('[yttt prototype state]', structuredClone(state));
}
function cycleVariant(direction) { const keys = Object.keys(variants); setVariant(keys[(keys.indexOf(state.variant) + direction + keys.length) % keys.length]); }
function addEvent(project, text, type = 'working') { state.events.unshift({ time: new Date().toLocaleTimeString('en-GB', { hour12: false }), project, text, type, source: '本地交互 · 模拟' }); }
function recordOperation(type, project, result, target) { state.operations.push({ id: `demo-op-${state.operations.length + 1}`, type, project, target, result, simulated: true }); }
function toast(message) { const element = document.getElementById('toast'); element.textContent = message; element.classList.add('visible'); clearTimeout(toastTimer); toastTimer = setTimeout(() => element.classList.remove('visible'), 4500); }
function answerQuestion(id, answer) {
  const question = state.questions.find(item => item.id === id);
  if (!question || question.status !== 'pending') { toast('这个演示问题已经处理，不会重复提交。'); return; }
  const project = getProject(question.project);
  if (offline(project)) { toast('Host 已失联，保留问题；请先在执行关系中恢复演示连接。'); return; }
  if (!answer.trim()) { toast('请先输入你的回答。'); return; }
  question.status = 'answered'; question.answer = answer.trim();
  if (!pendingQuestions(project.id).length) {
    project.status = 'working'; project.age = '刚刚'; project.progress = '刚刚';
    project.action = question.kind === 'approval' && answer === '拒绝' ? '授权已拒绝 · 评估无需权限的方案' : '收到决定 · 按确认范围继续推进';
    for (const child of project.children) { if (child.status === 'external') { child.status = 'working'; child.task = '收到主 Agent 决定，继续执行'; child.age = '刚刚'; } }
  }
  recordOperation('answer', project.id, 'simulated_acknowledged', question.request);
  addEvent(project.id, `回答已模拟确认：${answer.trim()}`, 'completed');
  if (dialog.open) dialog.close();
  const nextQuestion = visibleQuestions()[0];
  if (nextQuestion) { state.selectedQuestion = nextQuestion.id; state.selectedProject = nextQuestion.project; }
  render(); toast(`${project.name}：回答已在本地模拟确认。未发送给真实 Agent。`);
}
function showDialog(title, body, footer = '全部为演示数据 · 不连接真实 Agent', tabs = '') {
  document.getElementById('dialog-content').innerHTML = `<header class="dialog-header"><div><h2 id="dialog-title">${e(title)}</h2><small>yttt / COMMAND CENTER PROTOTYPE</small></div><button class="icon-button" data-action="close-dialog" aria-label="关闭详情">×</button></header>${tabs}<div class="dialog-body">${body}</div><footer class="dialog-footer"><span>${footer}</span><button class="secondary-button" data-action="close-dialog">关闭</button></footer>`;
  if (!dialog.open) dialog.showModal();
}
function showProject(id, tab = 'context') {
  const project = getProject(id); state.selectedProject = id; dialogTab = tab; render();
  const tabs = `<nav class="dialog-tabs" aria-label="项目详情页签">${[['context', '任务与执行'], ['terminal', '演示现场'], ['data', '观测数据']].map(([key, label]) => `<button class="${tab === key ? 'active' : ''}" data-action="project-tab" data-tab="${key}" data-project="${id}">${label}</button>`).join('')}</nav>`;
  let body;
  if (tab === 'terminal') {
    body = `<div class="terminal-label">演示执行记录，不是真实终端，也不会执行命令</div><pre class="terminal">${e(`[yttt] ${project.host} / ${project.branch}\n[scope] ${project.session} / run-${project.run}\n\n目标：${project.goal}\n阶段：${project.stages[project.stage]}\n当前：${project.action}\n\n${offline(project) ? '[connection] disconnected — 当前执行状态未知，以下是最后已知记录' : `[provider] ${providerNames[project.provider]} / ${project.model}`}\n${project.children.map(child => `[${child.status}] ${child.name}: ${child.task}`).join('\n')}\n\n${project.status === 'failed' ? '[error] signing credential unavailable\n[result] unsigned package retained; no release published' : project.status === 'completed' ? '[report] deliverable ready\n[acceptance] human review required' : '[observation] tool activity is not a completion percentage'}\n\n说明：以上为静态演示记录，用于验证“从大屏进入现场”的交互。`)}</pre>`;
  } else if (tab === 'data') {
    body = `<dl class="data-pairs"><dt>项目</dt><dd>${e(project.id)}</dd><dt>Host</dt><dd>${e(project.host)}</dd><dt>会话身份</dt><dd class="mono">${e(project.session)}</dd><dt>执行身份</dt><dd class="mono">${e(project.id)}/run-${project.run}</dd><dt>业务状态</dt><dd>${e(project.status)}</dd><dt>观测状态</dt><dd>${offline(project) ? '未知：Host 失联' : '演示连接正常'}</dd><dt>目标来源</dt><dd>模拟人工确认</dd><dt>阶段来源</dt><dd>模拟 Agent 上报；不是工具名推断</dd><dt>子任务口径</dt><dd>${project.done}/${project.total} 已确认子任务，不是耗时进度</dd><dt>费用与 token</dt><dd>未接入</dd><dt>控制能力</dt><dd>仅本页内存模拟</dd></dl>`;
  } else {
    body = `<div class="detail-grid"><div><section class="context-section"><div class="context-label">目标 · 模拟人工确认</div><h3 class="context-goal">${e(project.goal)}</h3><p class="context-paragraph" style="margin-top:10px">${e(project.context)}</p></section><section class="context-section"><div class="context-label">阶段 · 模拟 AGENT 上报</div>${steps(project)}</section><section class="context-section"><div class="context-label">验收条件</div><p class="context-paragraph">${e(project.acceptance)}</p></section><div class="operation-buttons">${operationButtons(project)}</div></div><div><section class="context-section"><div class="context-label">执行关系 · 包含已结束记录</div>${agentList(project)}</section><section class="context-section"><div class="context-label">交付证据</div><div class="evidence-row">${project.status === 'completed' ? '交付摘要已生成，等待人工核对内容。' : project.status === 'failed' ? '保留未签名构建产物；签名失败；没有发布结果。' : '当前阶段尚未提交最终交付结果。'}<small>证据引用为演示内容，不代表真实测试或发布结果。</small></div>${project.accepted ? '<div class="completed-answer">人工验收已在本地演示中记录。Agent 的轮次完成与验收状态分开保存。</div>' : ''}</section></div></div>`;
  }
  showDialog(`${project.name} · ${project.stages[project.stage]}`, body, '目标、阶段和执行关系均为场景数据；不会修改项目文件。', tabs);
}
function showQuestion(id) {
  const question = state.questions.find(item => item.id === id && item.status === 'pending');
  if (!question) { toast('这个问题已经处理。可在待处理中心查看已处理记录。'); return; }
  state.selectedProject = question.project; state.selectedQuestion = id; render();
  showDialog(`${getProject(question.project).name} · 待处理问题`, `${questionCard(question, true)}${replyEditor(question)}`, `问题绑定：${e(question.request)} · 回答只在本地模拟确认`);
}
function performOperation(action, id) {
  const project = getProject(id);
  if (action !== 'reconnect' && offline(project)) { toast('远端失联时，不能对这个演示执行提交操作。'); return; }
  if (action === 'reconnect') {
    state.disconnectedHosts = state.disconnectedHosts.filter(host => host !== project.host);
    addEvent(id, `${project.host} 连接已模拟恢复；原任务和待回答问题仍保留`);
    recordOperation('reconnect', id, 'simulated_connected', project.host);
    toast('演示连接已恢复。没有启动新会话，也没有自动回答问题。');
  } else if (action === 'retry' && project.status === 'failed') {
    project.run++; project.status = 'working'; project.action = '演示：修复凭据后，重新执行签名'; project.age = '刚刚'; project.progress = '刚刚';
    for (const child of project.children) { if (child.status === 'failed') { child.status = 'working'; child.task = '演示：重新执行签名验证'; child.age = '刚刚'; } }
    addEvent(id, `已模拟创建 run-${project.run}；上次签名失败保留在事件记录中`);
    recordOperation('retry', id, 'simulated_started', `${id}/run-${project.run}`);
    toast('已模拟修复后重试。未执行命令，未读取真实凭据。');
  } else if (action === 'accept' && project.status === 'completed' && !project.accepted) {
    project.accepted = true; project.done = project.total; project.action = '人工验收已记录 · 演示';
    addEvent(id, '人工验收通过，区别于 Agent 轮次结束', 'completed');
    recordOperation('accept', id, 'simulated_accepted', `${id}/run-${project.run}`);
    toast('已记录演示验收。验收计数和项目卡片已同步更新。');
  }
  render();
  if (dialog.open) showProject(id, dialogTab);
}
document.addEventListener('click', event => {
  const button = event.target.closest('[data-action]'); if (!button || button.disabled) return;
  const { action, project, question, tab } = button.dataset;
  if (action === 'variant') setVariant(button.dataset.variant);
  else if (action === 'previous-variant') cycleVariant(-1);
  else if (action === 'next-variant') cycleVariant(1);
  else if (action === 'attention') { state.attentionOnly = !state.attentionOnly; render(); }
  else if (action === 'reset') { if (dialog.open) dialog.close(); reset(state.scenario, true); }
  else if (action === 'project-detail') showProject(project);
  else if (action === 'terminal') showProject(project, 'terminal');
  else if (action === 'project-tab') showProject(project, tab);
  else if (action === 'question-detail') showQuestion(question);
  else if (action === 'answer') { const item = state.questions.find(item => item.id === question); answerQuestion(question, item.options[Number(button.dataset.option)]); }
  else if (action === 'custom-answer') answerQuestion(question, state.questions.find(item => item.id === question).draft);
  else if (action === 'close-dialog') dialog.close();
  else if (action === 'select-project') { state.selectedProject = project; state.selectedQuestion = pendingQuestions(project)[0]?.id || null; render(); }
  else if (action === 'select-question') { state.selectedQuestion = question; state.selectedProject = state.questions.find(item => item.id === question).project; render(); }
  else if (action === 'inbox-tab') { state.inboxTab = tab; render(); }
  else if (['reconnect', 'retry', 'accept'].includes(action)) performOperation(action, project);
  else if (action === 'execution-detail') {
    const item = getProject(project); const child = item.children.find(child => child.id === button.dataset.child);
    if (!child) showProject(project, 'terminal');
    else showDialog(`${item.name} / ${child.name}`, `<dl class="data-pairs"><dt>执行身份</dt><dd>${e(item.id)}/run-${item.run}/${e(child.id)}</dd><dt>父执行</dt><dd>${e(item.id)}/run-${item.run}/root</dd><dt>所属目标</dt><dd>${e(item.goal)}</dd><dt>负责事项</dt><dd>${e(child.task)}</dd><dt>当前观测</dt><dd>${badge(offline(item) ? 'unknown' : child.status, child.status === 'completed' && !offline(item) ? '已结束' : statusNames[offline(item) ? 'unknown' : child.status])}</dd><dt>控制能力</dt><dd>本原型仅查看派生执行，不独立取消子 Agent</dd></dl><div class="signal"><p>已结束的派生执行保留身份和结果；Host 失联时显示未知，不将缺失事件当成完成。</p></div>`);
  }
  else if (action === 'history') showDialog('最近变化 · 完整演示记录', `<div class="activity-rows">${state.events.map(eventRow).join('')}</div>`);
  else if (action === 'state') showDialog('原型状态 · 内存数据', `<pre class="state-json">${e(JSON.stringify(state, null, 2))}</pre>`, '刷新页面会重置所有交互状态；布局选择保留在 URL。');
  else if (action === 'info') showDialog('这个原型验证什么', `<ul class="info-list"><li><strong>A 项目矩阵：</strong>固定项目位置，快速巡视多个项目。</li><li><strong>B 待处理中心：</strong>以决策为中心，在问题旁边理解任务上下文。</li><li><strong>C 执行关系：</strong>查看主 Agent、派生执行、失败范围与观测健康。</li><li>项目、Agent、时间、问题、交付证据全部为<strong>演示数据</strong>。不采集现有工作台，也不调用任何真实控制接口。</li><li>按钮会改变当前页面内存状态。回答会减少待处理数量并更新项目状态；已处理问题、运维操作与事件会保留至重置。</li><li>使用底部切换器或左右方向键比较布局。输入框与详情弹窗中不会拦截方向键。</li><li>“Host 失联”场景保留问题并禁止提交；进入执行关系可以模拟恢复连接。</li><li>刷新会重置场景与操作，<span class="mono">?variant=A/B/C</span> 保留布局。该目录不参与 yttt 原生构建。</li></ul>`);
  else if (action === 'fullscreen') {
    if (document.fullscreenElement) document.exitFullscreen().catch(() => toast('当前浏览器不支持退出全屏。'));
    else if (document.documentElement.requestFullscreen) document.documentElement.requestFullscreen().catch(() => toast('当前浏览器未允许全屏，可使用浏览器的全屏菜单。'));
    else toast('当前浏览器不支持页面全屏，可使用浏览器的全屏菜单。');
  }
});
document.addEventListener('input', event => {
  if (event.target.id === 'search') { state.query = event.target.value; render(); }
  if (event.target.matches('[data-draft]')) { const question = state.questions.find(item => item.id === event.target.dataset.draft); if (question) question.draft = event.target.value; }
});
document.getElementById('scenario').addEventListener('change', event => reset(event.target.value, true));
document.addEventListener('keydown', event => {
  if (dialog.open || event.target.closest('input,textarea,select,[contenteditable]') || event.altKey || event.metaKey || event.ctrlKey) return;
  if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); cycleVariant(event.key === 'ArrowLeft' ? -1 : 1); }
});
window.addEventListener('popstate', () => { state.variant = requestedVariant(); render(); });
reset();
