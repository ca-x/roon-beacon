const copy = {
  en: {
    subtitle: 'Local discovery bridge for a remote Roon Core',
    signal: 'Signal Path',
    running: 'Running',
    local: 'Local LAN',
    remote: 'Remote Core',
    queries: 'SOOD Queries',
    live: 'Live Core',
    fallback: 'Fallbacks',
    tcp: 'TCP Sessions',
    runtime: 'Runtime',
    name: 'Fallback display name',
    interval: 'Announcement interval, seconds',
    timeout: 'Live SOOD timeout, ms',
    reverse: 'Reverse SOOD proxy',
    proxyIp: 'Reachable proxy IP',
    save: 'Save',
    saved: 'Saved',
    failed: 'Save failed',
    uptime: 'Uptime',
    broadcast: 'broadcast',
    multicast: 'multicast'
  },
  zh: {
    subtitle: '远端 Roon Core 的本地发现桥接',
    signal: '信号路径',
    running: '运行中',
    local: '本地局域网',
    remote: '远端 Core',
    queries: 'SOOD 查询',
    live: 'Core 真包',
    fallback: '模板兜底',
    tcp: 'TCP 会话',
    runtime: '运行设置',
    name: '兜底显示名',
    interval: '主动广播间隔，秒',
    timeout: '实时 SOOD 超时，毫秒',
    reverse: '反向 SOOD 代理',
    proxyIp: 'Core 可达的代理 IP',
    save: '保存',
    saved: '已保存',
    failed: '保存失败',
    uptime: '运行时间',
    broadcast: '广播',
    multicast: '多播'
  }
};

let lang = localStorage.getItem('roonBeaconLang') || (navigator.language.startsWith('zh') ? 'zh' : 'en');
const $ = (id) => document.getElementById(id);

function t(key) {
  return copy[lang][key] || copy.en[key] || key;
}

function applyLang() {
  document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
  document.querySelectorAll('[data-i18n]').forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });
  document.querySelectorAll('[data-lang]').forEach((btn) => {
    btn.classList.toggle('active', btn.dataset.lang === lang);
  });
}

function seconds(value) {
  const h = Math.floor(value / 3600);
  const m = Math.floor((value % 3600) / 60);
  const s = value % 60;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s}s`;
  return `${s}s`;
}

async function loadStatus(updateForm = false) {
  const res = await fetch('/api/status');
  const data = await res.json();

  $('localIp').textContent = data.local_ip;
  $('remoteIp').textContent = data.remote_ip;
  $('localPorts').textContent = `SOOD ${data.sood_port} · TCP ${data.discovery_port}`;
  $('mode').textContent = data.use_broadcast ? t('broadcast') : t('multicast');
  $('q').textContent = data.metrics.sood_queries;
  $('live').textContent = data.metrics.live_core_responses;
  $('fallback').textContent = data.metrics.template_fallbacks;
  $('tcp').textContent = data.metrics.tcp_connections;
  $('uptime').textContent = seconds(data.uptime_secs);

  if (updateForm) {
    $('name').value = data.config.name;
    $('broadcastInterval').value = data.config.broadcast_interval_secs;
    $('liveTimeout').value = data.config.live_proxy_timeout_ms;
    $('reverseProxy').checked = data.config.reverse_proxy;
    $('proxyIp').value = data.config.proxy_ip || '';
  }
}

document.querySelectorAll('[data-lang]').forEach((btn) => {
  btn.addEventListener('click', () => {
    lang = btn.dataset.lang;
    localStorage.setItem('roonBeaconLang', lang);
    applyLang();
    loadStatus(false).catch(() => {});
  });
});

$('configForm').addEventListener('submit', async (event) => {
  event.preventDefault();
  $('feedback').textContent = '';

  const body = {
    name: $('name').value,
    broadcast_interval_secs: Number($('broadcastInterval').value),
    live_proxy_timeout_ms: Number($('liveTimeout').value),
    reverse_proxy: $('reverseProxy').checked,
    proxy_ip: $('proxyIp').value || null
  };

  const res = await fetch('/api/config', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body)
  });

  $('feedback').textContent = res.ok ? t('saved') : t('failed');
  await loadStatus(true);
});

applyLang();
loadStatus(true).catch(() => {});
setInterval(() => loadStatus(false).catch(() => {}), 2000);
