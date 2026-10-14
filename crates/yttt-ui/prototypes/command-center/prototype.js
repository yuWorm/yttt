/* THROWAWAY: one fixed command screen, explicitly requested instead of variants.
 * Everything is simulated and in memory. No fetch, storage, shell or provider API.
 * Task lists, questions, conversations and execution identities share one state.
 */
'use strict';

const escapeHtml = value => String(value ?? '').replace(/[&<>"']/g, character => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
}[character]));
const names = { working: '进行中', waiting: '等你回答', external: '等依赖', failed: '失败', completed: '已结束', accepted: '已验收', planned: '待开始', unknown: '未知', review: '待验收' };
const symbols = { working: '●', waiting: '?', external: '◷', failed: '!', completed: '✓', accepted: '✓', planned: '○', unknown: '◇', review: '□' };
// title, provider, business state, duration, execution identity, current action
const projectSeeds = [
  {
    id: 'yttt', name: 'yttt', goal: '多项目指挥台', phase: '交互预览', host: 'mac-local',
    acceptance: '一屏看清主要任务；回答不跳页；选中任务时全局仍然可见。',
    tasks: [
      ['单屏任务矩阵与联动', 'OMP', 'working', '12m', 'root', '连接任务、问题和关联对话'],
      ['高密度任务行排版', 'Claude', 'working', '8m', 'layout', '调整状态、执行者与时间的对齐'],
      ['验证问题回答归属', 'OMP', 'planned', '—', 'root', '等待界面交互准备完成'],
      ['梳理任务观测事件', 'OMP', 'completed', '4m', 'events', '已整理目标、阻塞与提问事件'],
      ['统一项目分区位置', 'Codex', 'completed', '7m', 'visual', '固定矩阵位置，不随状态重排'],
      ['回放失联与恢复场景', 'OMP', 'planned', '—', 'root', '覆盖失联时保留问题的行为'],
    ],
  },
  {
    id: 'atlas', name: 'Atlas', goal: '账户体系 · 接口迁移', phase: '接口实现', host: 'mac-local',
    acceptance: '兼容范围有明确决定；新接口可登录；迁移可以回滚。',
    tasks: [
      ['旧客户端兼容策略', 'Claude', 'waiting', '4m', 'root', '等你决定 v1 接口是否保留'],
      ['迁移登录路由', 'Codex', 'external', '4m', 'api', '等待兼容策略，不需要重复提问'],
      ['新接口集成用例', 'OMP', 'completed', '6m', 'tests', '新接口用例已完成，尚非全量验收'],
      ['统一账户身份字段', 'Claude', 'completed', '8m', 'root', '已确认字段映射'],
      ['保留迁移回滚路径', 'Codex', 'completed', '3m', 'rollback', '已整理回滚步骤'],
      ['兼容范围回归验证', 'Claude', 'planned', '—', 'root', '等待兼容方案确定'],
    ],
  },
  {
    id: 'gateway', name: 'Gateway', goal: '网关鉴权 · 灰度验证', phase: '环境验证', host: 'build-linux',
    acceptance: '鉴权日志无回归；本次只读验证；生产发布另行授权。',
    tasks: [
      ['读取预发布鉴权日志', 'OMP', 'waiting', '2m', 'root', '等待一次性只读授权'],
      ['追踪灰度请求链路', 'Claude', 'external', '2m', 'trace', '等待日志查询结果'],
      ['修复令牌路由分支', 'Codex', 'completed', '9m', 'fix', '修复已完成，等待环境验证'],
      ['复现鉴权失败请求', 'OMP', 'completed', '6m', 'root', '已保留复现条件'],
      ['校验本地回归用例', 'Codex', 'completed', '3m', 'tests', '本地用例通过'],
      ['确认生产发布范围', 'OMP', 'planned', '—', 'root', '不随日志授权自动发布'],
    ],
  },
  {
    id: 'studio', name: 'Studio', goal: '编辑器 · 大文件滚动', phase: '性能验证', host: 'mac-local',
    acceptance: '减少滚动重复布局；光标与选择行为不变；按相同样本对比。',
    tasks: [
      ['执行滚动性能基准', 'OMP', 'working', '6m', 'benchmark', '使用相同长文档对比滚动延迟'],
      ['减少重复布局计算', 'Codex', 'working', '23m', 'root', '更新可见区域的布局缓存'],
      ['采集优化前基线', 'OMP', 'completed', '10m', 'baseline', '基线样本已保留'],
      ['检查文本选择行为', 'Claude', 'completed', '4m', 'review', '已核对拖选与光标定位'],
      ['低配设备回归', 'OMP', 'planned', '—', 'benchmark', '等待基准对比结束'],
      ['整理性能交付记录', 'Codex', 'planned', '—', 'root', '交付结论需要关联实际样本'],
    ],
  },
  {
    id: 'mobile', name: 'Mobile', goal: '移动端 · 会话恢复', phase: '恢复交互', host: 'mac-local',
    acceptance: '草稿不丢失；重连不自动发送；已运行的任务不重复执行。',
    tasks: [
      ['确认未发送草稿行为', 'Claude', 'waiting', '1m', 'root', '等待选择重连后的草稿处理方式'],
      ['真机重连流程验证', 'Codex', 'external', '1m', 'device', '等待恢复交互确认'],
      ['保存本地输入草稿', 'Codex', 'completed', '8m', 'storage', '草稿保存与实际发送分离'],
      ['区分恢复与重新执行', 'Claude', 'completed', '5m', 'root', '恢复不创建新执行实例'],
      ['保留 Host 会话身份', 'OMP', 'completed', '7m', 'identity', '已对齐会话与执行身份'],
      ['验证多次断线恢复', 'Claude', 'planned', '—', 'root', '覆盖连续断线与输入保留'],
    ],
  },
  {
    id: 'release', name: 'Release', goal: 'Windows · 安装包发布', phase: '签名构建', host: 'win-runner',
    acceptance: '安装可用；签名结果明确；校验和匹配；未经确认不发布。',
    tasks: [
      ['安装包签名失败', 'OMP', 'failed', '7m', 'signer', '签名凭据不可用；没有发布'],
      ['核对发布校验和', 'OMP', 'external', '7m', 'root', '等待签名产物'],
      ['生成未签名安装包', 'Codex', 'completed', '13m', 'pack', '构建产物保留，不代表已发布'],
      ['校对构建流水线', 'Claude', 'completed', '5m', 'workflow', '发布步骤与构建步骤分离'],
      ['安装与卸载验证', 'Codex', 'planned', '—', 'tester', '等待待验证安装包'],
      ['确认并上传发布资产', 'OMP', 'planned', '—', 'root', '需通过前置验证后再确认'],
    ],
  },
  {
    id: 'docs', name: 'Docs', goal: '文档 · 远端接入指南', phase: '人工验收', host: 'mac-local',
    acceptance: '按指南可完成首次接入；风险提示完整；引用链接可访问。',
    tasks: [
      ['远端接入指南验收', 'Claude', 'review', '18m', 'root', '交付摘要已就绪，等待你验收'],
      ['编写首次连接步骤', 'Claude', 'completed', '11m', 'root', '覆盖 Host 准备与客户端连接'],
      ['解释指纹确认风险', 'OMP', 'completed', '6m', 'security', '已补充一次信任和密钥变化提示'],
      ['校对断线恢复说明', 'Claude', 'completed', '8m', 'root', '已区分恢复会话和启动新会话'],
      ['检查引用链接', 'Codex', 'completed', '3m', 'links', '演示：引用已检查'],
      ['同步中英文术语', 'Claude', 'completed', '9m', 'language', '已统一 Host、会话与执行用语'],
    ],
  },
  {
    id: 'pipeline', name: 'Pipeline', goal: 'CI · 发布资产校验', phase: '等待构建', host: 'build-linux',
    acceptance: '所有目标平台的产物齐全；校验和匹配；不把等待误报失败。',
    tasks: [
      ['等待 CI 上传产物', 'OMP', 'external', '11m', 'root', '等待外部构建，不需要你回答'],
      ['核对各平台资产清单', 'Codex', 'external', '11m', 'assets', '等待远端文件出现'],
      ['整理目标平台列表', 'OMP', 'completed', '4m', 'root', '目标平台列表已确认'],
      ['生成校验和检查步骤', 'Codex', 'completed', '6m', 'assets', '核对输入为最终构建产物'],
      ['检查下载地址', 'OMP', 'planned', '—', 'root', '等待发布资产准备完成'],
      ['输出资产验证结论', 'Codex', 'planned', '—', 'assets', '结果需要关联具体产物'],
    ],
  },
  {
    id: 'design', name: 'Design System', goal: '组件库 · 键盘可访问性', phase: '焦点修复', host: 'mac-local',
    acceptance: 'Tab 顺序可预测；Escape 正确退出；焦点返回触发元素。',
    tasks: [
      ['修复关闭后的焦点返回', 'Codex', 'working', '31m', 'root', '定位对话框关闭后的焦点归属'],
      ['审计菜单键盘顺序', 'Claude', 'working', '9m', 'audit', '检查菜单与子菜单的焦点移动'],
      ['列出可访问性缺口', 'Claude', 'completed', '8m', 'audit', '区分对话框问题与菜单问题'],
      ['统一焦点可见样式', 'OMP', 'completed', '5m', 'style', '键盘焦点必须可见'],
      ['验证全键盘操作', 'OMP', 'planned', '—', 'tests', '在交互修复完成后执行'],
      ['交付键盘行为说明', 'Codex', 'planned', '—', 'root', '解释新增交互与保留行为'],
    ],
  },
];
const questionSeeds = [
  {
    id: 'Q01', project: 'atlas', task: 0, phase: '接口迁移', title: '旧版客户端，是否保留 v1 兼容？',
    context: '保留兼容层无需立即升级客户端；仅新接口则需要同步发布。',
    impact: '迁移登录路由与后续回归范围都依赖这个决定。',
    options: ['保留一个版本', '仅支持新接口'], age: '4m', blockedTasks: [1],
  },
  {
    id: 'Q02', project: 'gateway', task: 0, phase: '灰度验证', title: '允许读取预发布环境最近 15 分钟的日志？',
    context: '只读一次，不改配置、不部署；授权只绑定这一次查询。',
    impact: '请求追踪正在等待日志结果；生产发布仍需另行确认。',
    options: ['允许这一次', '拒绝'], age: '2m', blockedTasks: [1],
  },
  {
    id: 'Q03', project: 'mobile', task: 0, phase: '恢复交互', title: '断线重连后，如何处理未发送的输入？',
    context: '本地草稿可能更新，自动提交可能导致原任务重复执行。',
    impact: '真机验证正在等待恢复交互确认。',
    options: ['保留草稿，手动发送', '重连后提示确认'], age: '1m', blockedTasks: [1],
  },
];
const extraQuestions = [
  { id: 'Q04', project: 'studio', task: 0, phase: '性能验证', title: '性能回归按哪个设备档位验收？', context: '开发机与最低支持设备的验收阈值不同。', impact: '决定基准覆盖范围。', options: ['最低支持设备', '覆盖两档设备'], age: '刚刚', blockedTasks: [] },
  { id: 'Q05', project: 'design', task: 0, phase: '焦点修复', title: '这次是否同时修复菜单交互？', context: '菜单问题与对话框相似，但属于独立交互。', impact: '决定这次交付范围。', options: ['只修复对话框', '菜单另建任务'], age: '刚刚', blockedTasks: [] },
];
let state;
const $ = id => document.getElementById(id);
const projectById = id => state.projects.find(project => project.id === id);
const isOffline = project => state.disconnectedHosts.includes(project.host);
const currentStatus = (project, task) => isOffline(project) && !['completed', 'accepted', 'planned'].includes(task.status) ? 'unknown' : task.status;
const questionFor = (projectId, taskIndex) => state.questions.find(question => question.project === projectId && question.task === taskIndex);
const activeQuestionFor = (projectId, taskIndex) => state.questions.find(question => question.project === projectId && question.task === taskIndex && question.status === 'pending');
const selectedTask = () => projectById(state.selection.project).tasks[state.selection.task];
const stateClass = status => status === 'review' ? 'waiting' : status;
const selectionKey = (project, task) => `${project.id}/${task.index}`;

function reset(scenario = 'normal') {
  state = {
    scenario,
    projects: structuredClone(projectSeeds).map((project, projectIndex) => ({
      ...project,
      code: `G${projectIndex + 1}`,
      tasks: project.tasks.map(([title, provider, status, age, actor, action], index) => ({
        title, provider, status, age, actor, action, index, run: 1, code: `G${projectIndex + 1}-${String(index + 1).padStart(2, '0')}`,
        messages: [],
      })),
    })),
    questions: structuredClone([...questionSeeds, ...(scenario === 'questions' ? extraQuestions : [])]).map(question => ({ ...question, status: 'pending', answer: null })),
    selection: { project: scenario === 'offline' ? 'gateway' : 'atlas', task: 0 },
    disconnectedHosts: scenario === 'offline' ? ['build-linux'] : [],
    drafts: {}, operations: [],
    lastEvent: '54 条任务直接展开；点击任意任务，只更新下方关联对话。',
  };
  for (const question of state.questions) projectById(question.project).tasks[question.task].status = 'waiting';
  $('scenario').value = scenario;
  $('message').dataset.target = '';
  render();
}

function executions() {
  const records = [];
  for (const project of state.projects) {
    const seen = new Set();
    for (const task of project.tasks) {
      if (!['working', 'waiting', 'external', 'failed'].includes(task.status) || seen.has(task.actor)) continue;
      seen.add(task.actor);
      records.push({ project, task, status: currentStatus(project, task) });
    }
  }
  return records;
}

function renderTotals() {
  const agents = executions();
  const pending = state.questions.filter(question => question.status === 'pending').length;
  const failures = agents.filter(agent => agent.status === 'failed').length;
  const unknown = agents.filter(agent => agent.status === 'unknown').length;
  $('totals').innerHTML = `<span>项目<b>${state.projects.length}</b></span><span>任务<b>${state.projects.reduce((sum, project) => sum + project.tasks.length, 0)}</b></span><span>运行<b>${agents.filter(agent => agent.status === 'working').length}</b></span><span class="waiting">等你回答<b>${pending}</b></span><span class="failed">失败<b>${failures}</b></span><span>观测未知<b>${unknown}</b></span>`;
  $('pending-count').textContent = pending;
}

function renderProject(project) {
  const done = project.tasks.filter(task => ['completed', 'accepted'].includes(task.status)).length;
  const pending = state.questions.filter(question => question.project === project.id && question.status === 'pending').length;
  const selected = project.id === state.selection.project;
  const states = project.tasks.map(task => currentStatus(project, task));
  const running = states.filter(status => status === 'working').length;
  const footer = isOffline(project) ? '<span class="unknown">Host 失联 · 未知不等于失败</span>' : pending ? `<span class="waiting">${pending} 个问题等你回答</span>` : states.includes('failed') ? '<span class="failed">签名失败 · 未发布</span>' : states.includes('review') ? '<span class="waiting">交付已就绪 · 等待验收</span>' : states.includes('external') ? '<span class="external">等外部依赖，不需要回答</span>' : `<span>进行 ${running} · 待开始 ${states.filter(status => status === 'planned').length}</span>`;
  return `<article class="project ${selected ? 'selected' : ''}" data-project="${project.id}" aria-label="${escapeHtml(project.name)} 项目任务">
    <header class="project-head"><h2><span class="group-code">${project.code}</span>${escapeHtml(project.name)}</h2><span class="host">${escapeHtml(project.host)}</span></header>
    <div class="project-goal"><span class="goal">${escapeHtml(project.goal)}</span><span class="phase">${escapeHtml(project.phase)}</span></div>
    <div class="task-track" aria-hidden="true">${project.tasks.map(task => {
      const status = currentStatus(project, task);
      return `<span class="task-tile ${stateClass(status)}" title="${escapeHtml(task.title)} · ${names[status]}"><span class="tile-code">${String(task.index + 1).padStart(2, '0')}</span><span>${['completed', 'accepted'].includes(status) ? '✓' : status === 'planned' ? '—' : status === 'unknown' ? '?' : task.age}</span></span>`;
    }).join('')}</div>
    <div class="project-summary"><span class="summary-left">完成 ${done}/${project.tasks.length}</span><span class="summary-right">${footer}</span></div>
    <div class="task-list" data-scroll="${project.id}">${project.tasks.map(task => {
      const status = currentStatus(project, task);
      const isSelected = selected && task.index === state.selection.task;
      return `<button class="task-row ${stateClass(status)} ${isSelected ? 'selected' : ''}" data-action="select-task" data-project="${project.id}" data-task="${task.index}" aria-pressed="${isSelected}" aria-label="${task.code} ${escapeHtml(task.title)}，${names[status]}，${task.provider}" title="${escapeHtml(task.action)}">
        <span class="symbol">${symbols[status]}</span><span class="task-title"><span class="task-code">${String(task.index + 1).padStart(2, '0')}</span>${escapeHtml(task.title)}</span><span class="task-owner">${escapeHtml(task.provider)}</span><span class="task-age">${escapeHtml(status === 'planned' ? '待开始' : status === 'unknown' ? '未知' : task.age)}</span>
      </button>`;
    }).join('')}</div>
  </article>`;
}

function renderQuestions() {
  $('questions').innerHTML = state.questions.map(question => {
    const project = projectById(question.project);
    const task = project.tasks[question.task];
    const answered = question.status === 'answered';
    const disconnected = isOffline(project);
    const selected = state.selection.project === project.id && state.selection.task === task.index;
    return `<article class="question ${answered ? 'answered' : ''} ${selected ? 'selected' : ''}" data-question="${question.id}">
      <button class="question-heading" data-action="select-question" data-question="${question.id}"><span class="question-code">${question.id} / ${task.code}</span><span class="question-project">${escapeHtml(project.name)} · ${escapeHtml(question.phase)}</span><span class="question-age">${answered ? '✓ 已回答' : `已等 ${question.age}`}</span></button>
      <h3 class="question-title">${escapeHtml(question.title)}</h3>
      <p class="question-context">${answered ? `你的回答：${escapeHtml(question.answer)}` : disconnected ? '<span class="offline-warning">Host 已失联，问题保留。恢复连接前不能提交回答。</span>' : escapeHtml(question.context)}</p>
      ${answered ? '<div class="answer-receipt">✓ 本地模拟已确认 · 原任务继续，记录不消失</div>' : `<div class="question-answers">${question.options.map((option, index) => `<button class="answer ${index ? 'secondary' : ''}" data-action="answer" data-question="${question.id}" data-option="${index}" ${disconnected ? 'disabled' : ''}>${escapeHtml(option)}</button>`).join('')}<button class="question-reply" data-action="compose-question" data-question="${question.id}" ${disconnected ? 'disabled' : ''}>补充回答 →</button></div>`}
    </article>`;
  }).join('');
}

function conversationMessages(project, task) {
  const question = questionFor(project.id, task.index);
  const messages = [
    { role: 'user', time: '15:02', speaker: '你', text: `推进「${task.title}」。${project.acceptance}` },
    { role: 'agent', time: '15:08', speaker: task.provider, text: `${task.action}。${task.actor === 'root' ? '这是当前主会话负责的任务。' : `由主会话派生的 ${task.actor} 执行负责。`}` },
  ];
  if (question) {
    messages.push({ role: 'agent', time: '15:12', speaker: task.provider, text: `${question.title}\n${question.context} ${question.impact}` });
  } else if (task.status === 'failed') {
    messages.push({ role: 'agent', time: '15:12', speaker: task.provider, text: '签名凭据不可用。未签名安装包已保留，未上传、未发布。这里只模拟修复后的重试，不访问真实凭据。' });
  } else if (task.status === 'review') {
    messages.push({ role: 'agent', time: '15:12', speaker: task.provider, text: '演示交付：接入步骤、风险提示和恢复说明已整理。我的轮次结束不代表人工验收通过，请核对验收条件。' });
  } else if (task.status === 'external') {
    messages.push({ role: 'system', time: '15:12', speaker: '状态', text: '等待外部依赖，不进入人的待回答队列。没有新输出不能单独证明执行失败。' });
  } else {
    messages.push({ role: 'system', time: '15:12', speaker: '状态', text: `${names[task.status]}。此处保留所选任务的对话，其他项目的位置与内容保持可见。` });
  }
  messages.push(...task.messages);
  if (isOffline(project)) messages.push({ role: 'system', time: '现在', speaker: '观测', text: 'Host 连接中断。以上是最后已知记录；当前执行未知。任务与未回答问题继续保留，写操作已禁用。' });
  return messages;
}

function renderConversation() {
  const project = projectById(state.selection.project);
  const task = selectedTask();
  const status = currentStatus(project, task);
  $('conversation-ref').textContent = `${task.code} / ${task.provider}`;
  $('conversation-context').innerHTML = `<strong>${escapeHtml(task.title)}</strong><span class="context-state ${stateClass(status)}">${names[status]}</span><span>${escapeHtml(project.name)} / ${escapeHtml(project.host)} / ${task.actor === 'root' ? '主会话' : `派生 ${escapeHtml(task.actor)}`} / run-${task.run}</span>`;
  $('transcript').innerHTML = conversationMessages(project, task).map(message => `<div class="message ${message.role}"><span class="time">${escapeHtml(message.time)}</span><div><span class="speaker">${escapeHtml(message.speaker)}:</span><span class="message-text">${escapeHtml(message.text)}</span></div></div>`).join('');
  let operation = '';
  if (isOffline(project)) operation = '<button class="operation" data-action="reconnect">模拟恢复连接</button><span class="operation-note">不启动新执行，不自动回答</span>';
  else if (task.status === 'failed') operation = '<button class="operation" data-action="retry">模拟修复后重试</button><span class="operation-note">保留失败记录，新建执行编号</span>';
  else if (task.status === 'review') operation = '<button class="operation" data-action="accept">模拟验收通过</button><span class="operation-note">验收与 Agent 结束分开记录</span>';
  $('conversation-actions').innerHTML = operation;
  const question = activeQuestionFor(project.id, task.index);
  const target = question ? `question:${question.id}` : `task:${selectionKey(project, task)}`;
  $('reply-target').textContent = question ? `回复 ${question.id} · ${project.name} / ${task.code} · 绑定这个问题` : `补充给 ${project.name} / ${task.code} · ${task.actor === 'root' ? '主会话' : `派生 ${task.actor}`}`;
  const input = $('message');
  if (input.dataset.target !== target) { input.value = state.drafts[target] || ''; input.dataset.target = target; }
  input.disabled = isOffline(project);
  input.placeholder = isOffline(project) ? 'Host 失联，恢复连接后才能回复' : question ? '补充你的决定，直接回答这个问题…' : '给所选任务补充一句…';
  $('send').disabled = isOffline(project);
  $('send').textContent = question ? '回答' : '发送';
}

function renderAgents() {
  const agents = executions();
  $('agent-count').textContent = agents.length;
  $('agents').innerHTML = agents.map(({ project, task, status }) => {
    const selected = state.selection.project === project.id && state.selection.task === task.index;
    return `<button class="agent-row ${selected ? 'selected' : ''}" data-action="select-task" data-project="${project.id}" data-task="${task.index}" title="${escapeHtml(task.title)} · ${escapeHtml(task.action)}" aria-label="定位 ${task.code} ${task.provider} ${escapeHtml(task.title)}">
      <span class="agent-identity"><span class="agent-provider">${escapeHtml(task.provider)}</span><span>${task.code}</span><span class="agent-scope">${task.actor === 'root' ? '主' : '↳派生'}</span></span><span class="agent-state ${stateClass(status)}">${status === 'waiting' ? '等你' : names[status]}</span><span class="agent-time">${status === 'unknown' ? '—' : escapeHtml(task.age)}</span>
    </button>`;
  }).join('');
}

function render() {
  const scrolls = [...document.querySelectorAll('[data-scroll]')].map(element => [element.dataset.scroll, element.scrollTop]);
  renderTotals();
  $('board').innerHTML = state.projects.map(renderProject).join('');
  for (const [id, position] of scrolls) {
    const element = document.querySelector(`[data-scroll="${id}"]`);
    if (element) element.scrollTop = position;
  }
  renderQuestions();
  renderConversation();
  renderAgents();
  $('last-event').textContent = state.lastEvent;
  window.prototypeState = state;
}

function selectTask(projectId, taskIndex, focusComposer = false) {
  state.selection = { project: projectId, task: Number(taskIndex) };
  state.lastEvent = `已选 ${selectedTask().code} · ${selectedTask().title}；项目矩阵与待回答问题保持原位。`;
  render();
  $('transcript').scrollTop = 0;
  if (focusComposer) $('message').focus();
}

function addMessage(task, role, text) {
  task.messages.push({ role, time: new Date().toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', hour12: false }), speaker: role === 'user' ? '你' : role === 'system' ? '状态' : task.provider, text });
}

function recordOperation(type, project, task, result, questionId = null) {
  state.operations.push({ id: `demo-${state.operations.length + 1}`, type, project: project.id, task: task.code, execution: `${project.id}/${task.actor}/run-${task.run}`, questionId, result, simulated: true });
}

function answerQuestion(questionId, answer) {
  const question = state.questions.find(item => item.id === questionId);
  if (!question || question.status !== 'pending') return;
  const project = projectById(question.project);
  const task = project.tasks[question.task];
  if (isOffline(project) || !answer.trim()) return;
  question.status = 'answered';
  question.answer = answer.trim();
  task.status = 'working'; task.age = '刚刚';
  task.action = answer === '拒绝' ? '授权被拒绝，整理无需日志权限的验证方案' : '已收到决定，按确认范围继续推进';
  addMessage(task, 'user', answer.trim());
  addMessage(task, 'agent', `本地模拟已确认：${task.action}。没有向真实 Agent 发送内容。`);
  for (const index of question.blockedTasks) {
    const child = project.tasks[index];
    child.status = answer === '拒绝' ? 'planned' : 'working';
    child.age = answer === '拒绝' ? '—' : '刚刚';
    child.action = answer === '拒绝' ? '授权未获准，等待替代验证方案' : '收到前置决定，继续负责的任务';
  }
  recordOperation('answer', project, task, 'simulated_acknowledged', question.id);
  state.drafts[`question:${question.id}`] = '';
  state.selection = { project: project.id, task: task.index };
  state.lastEvent = `${question.id} 已模拟回答「${answer.trim()}」；问题原位留档，没有跳到其他项目。`;
  render();
  $('transcript').scrollTop = $('transcript').scrollHeight;
}

function submitMessage() {
  const project = projectById(state.selection.project);
  const task = selectedTask();
  const text = $('message').value.trim();
  if (!text || isOffline(project)) return;
  const question = activeQuestionFor(project.id, task.index);
  const target = question ? `question:${question.id}` : `task:${selectionKey(project, task)}`;
  if ($('message').dataset.target !== target) return;
  if (question) { answerQuestion(question.id, text); return; }
  addMessage(task, 'user', text);
  addMessage(task, 'system', '补充说明已记入这个任务的本地演示对话；未发送给真实 Agent。');
  recordOperation('message', project, task, 'simulated_recorded');
  state.drafts[target] = '';
  $('message').value = '';
  state.lastEvent = `补充说明已绑定 ${task.code} / ${project.name}，其他任务不受影响。`;
  render();
  $('transcript').scrollTop = $('transcript').scrollHeight;
}

function performOperation(action) {
  const project = projectById(state.selection.project);
  const task = selectedTask();
  if (action !== 'reconnect' && isOffline(project)) return;
  if (action === 'reconnect') {
    state.disconnectedHosts = state.disconnectedHosts.filter(host => host !== project.host);
    addMessage(task, 'system', '演示连接已恢复；执行编号保持不变，未自动回答任何问题。');
    recordOperation('reconnect', project, task, 'simulated_connected');
    state.lastEvent = `${project.host} 已模拟恢复；原任务、执行身份和未回答问题均保留。`;
  } else if (action === 'retry' && task.status === 'failed') {
    addMessage(task, 'system', `run-${task.run} 失败原因：签名凭据不可用。此失败记录保留。`);
    task.run++; task.status = 'working'; task.age = '刚刚'; task.title = '重新执行安装包签名';
    task.action = '演示：凭据修复后重新执行签名，尚未发布';
    addMessage(task, 'system', `已模拟启动 run-${task.run}；未读取真实凭据，未执行真实命令。`);
    recordOperation('retry', project, task, 'simulated_started');
    state.lastEvent = `${task.code} 已模拟重试；上次失败留在对话，新执行为 run-${task.run}。`;
  } else if (action === 'accept' && task.status === 'review') {
    task.status = 'accepted'; task.action = '人工验收已在本地演示中确认';
    addMessage(task, 'user', '验收通过。');
    addMessage(task, 'system', '人工验收单独记录，不由 Agent 轮次结束推断。');
    recordOperation('accept', project, task, 'simulated_accepted');
    state.lastEvent = `${task.code} 验收已模拟记录，完成计数同步更新。`;
  }
  render();
  $('transcript').scrollTop = $('transcript').scrollHeight;
}

document.addEventListener('click', event => {
  const button = event.target.closest('[data-action]');
  if (!button || button.disabled) return;
  const { action, project, task, question } = button.dataset;
  if (action === 'select-task') selectTask(project, task);
  else if (action === 'select-question' || action === 'compose-question') {
    const item = state.questions.find(item => item.id === question);
    selectTask(item.project, item.task, action === 'compose-question');
  } else if (action === 'answer') {
    const item = state.questions.find(item => item.id === question);
    answerQuestion(question, item.options[Number(button.dataset.option)]);
  } else if (['reconnect', 'retry', 'accept'].includes(action)) performOperation(action);
  else if (action === 'reset') reset(state.scenario);
  else if (action === 'fullscreen') {
    if (document.fullscreenElement) document.exitFullscreen();
    else if (document.documentElement.requestFullscreen) document.documentElement.requestFullscreen().catch(() => { $('last-event').textContent = '浏览器未允许页面全屏，可使用浏览器全屏菜单。'; });
  }
});
$('scenario').addEventListener('change', event => reset(event.target.value));
$('composer').addEventListener('submit', event => { event.preventDefault(); submitMessage(); });
$('message').addEventListener('input', event => { state.drafts[event.target.dataset.target] = event.target.value; });
document.addEventListener('keydown', event => {
  const editing = event.target.closest('textarea,input,select,[contenteditable]');
  if (editing) {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && event.target.id === 'message') { event.preventDefault(); submitMessage(); }
    if (event.key === 'Escape' && event.target.id === 'message') { event.target.blur(); }
    return;
  }
  if (event.ctrlKey || event.metaKey || event.altKey) return;
  if (event.key.toLowerCase() === 'r') { event.preventDefault(); $('message').focus(); return; }
  if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
    event.preventDefault();
    const tasks = state.projects.flatMap(project => project.tasks.map(task => ({ project: project.id, task: task.index })));
    const index = tasks.findIndex(item => item.project === state.selection.project && item.task === state.selection.task);
    const next = tasks[(index + (event.key === 'ArrowDown' ? 1 : -1) + tasks.length) % tasks.length];
    selectTask(next.project, next.task);
    document.querySelector(`.task-row[data-project="${next.project}"][data-task="${next.task}"]`).focus({ preventScroll: true });
  }
});
reset();
