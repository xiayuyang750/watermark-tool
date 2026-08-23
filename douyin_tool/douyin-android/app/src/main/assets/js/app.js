/* 抖音 + X 多平台下载器前端逻辑
 * 说明：
 *  - 真正的解析在 Android 端（DownloadBridge.processShareText）完成，
 *    前端只负责：收集输入 → 交给 Android → 渲染状态/结果。
 *  - 解析历史存 localStorage（本地，无后端），写/删/清空/重新解析/复制都在这。
 *  - 展示逻辑（视频封面点击播放 / 图片直接展示 / 历史点击展开）都在这里。
 *  - 后续新增平台/按钮，优先改 PLATFORMS 与 handleParse。
 */

'use strict';

var videoUrl = '';
var currentKind = 'video';      // 当前结果类型：video / image
var currentCover = '';
var currentFiles = [];          // 文件列表 [{url, kind, cover}]
var HISTORY_KEY = 'download_history';
var HISTORY_LIMIT = 50;

/** 平台识别：输入文本里出现这些域名就判定为对应平台。 */
var PLATFORMS = [
  { key: 'douyin', name: '抖音', hosts: ['douyin.com', 'iesdouyin.com', 'v.douyin'] },
  { key: 'x', name: 'X', hosts: ['x.com', 'twitter.com', 'vxtwitter.com', 'fxtwitter.com', 'nitter.net'] }
];

function detectPlatform(text) {
  for (var i = 0; i < PLATFORMS.length; i++) {
    for (var j = 0; j < PLATFORMS[i].hosts.length; j++) {
      if (text.indexOf(PLATFORMS[i].hosts[j]) !== -1) return PLATFORMS[i].name;
    }
  }
  return null;
}

// 平台名 -> 存储桶 key（供历史分桶用）
function platformKey(name) {
  for (var i = 0; i < PLATFORMS.length; i++) {
    if (PLATFORMS[i].name === name) return PLATFORMS[i].key;
  }
  return 'douyin'; // 未知默认归到抖音（仅在旧数据兼容时用）
}

// ==================== 历史存储（按平台分桶） ====================

// 存储结构: { douyin:[...], x:[...] }。Android/本地都整体存这份 JSON。
function loadAllHistory() {
  var raw;
  if (window.DownloadBridge && window.DownloadBridge.loadHistory) {
    raw = window.DownloadBridge.loadHistory() || '{}';
  } else {
    raw = localStorage.getItem(HISTORY_KEY) || '{}';
  }
  try {
    var obj = JSON.parse(raw);
    // 兼容旧数据（旧格式是数组） -> 迁移成平台分桶
    if (Array.isArray(obj)) {
      var migrated = {};
      obj.forEach(function(h) {
        var k = (h.platform === 'X' || h.platform === 'x') ? 'x' : 'douyin';
        (migrated[k] = migrated[k] || []).push(h);
      });
      return migrated;
    }
    return obj && typeof obj === 'object' ? obj : {};
  } catch (e) { return {}; }
}

function saveAllHistory(obj) {
  var json = JSON.stringify(obj);
  if (window.DownloadBridge && window.DownloadBridge.saveHistory) {
    window.DownloadBridge.saveHistory(json);
  } else {
    try { localStorage.setItem(HISTORY_KEY, json); } catch (e) {}
  }
}

// 读取某平台的列表（platform 为英文 key: douyin / x）
function loadHistory(platform) {
  var all = loadAllHistory();
  var arr = all[platform];
  return Array.isArray(arr) ? arr : [];
}

function saveHistory(platform, arr) {
  var all = loadAllHistory();
  all[platform] = arr || [];
  saveAllHistory(all);
}

var expandedId = null;  // 当前展开的历史项唯一 id（对齐桌面版扩展逻辑，只开一条）

function renderHistory() {
  var arr = loadHistory(currentPlatform);
  var empty = document.getElementById('historyEmpty');
  var list = document.getElementById('historyList');
  empty.style.display = arr.length ? 'none' : 'block';
  list.innerHTML = '';
  for (var i = 0; i < arr.length; i++) {
    var it = arr[i];
    if (!it.id) it.id = 'h' + i + '_' + Date.now();
    var item = document.createElement('div');
    item.className = 'history-item' + (expandedId === it.id ? ' expanded' : '');
    item.setAttribute('data-id', it.id);
    var thumb = it.cover
      ? '<img src="' + mediaSrc(it.cover) + '" onerror="this.parentNode.innerHTML=\'<span class=ph>▶</span>\'">'
      : '<span class="ph">▶</span>';
    item.innerHTML =
      '<div class="history-headrow">' +
        '<div class="history-thumb">' + thumb + '</div>' +
        '<div class="history-info">' +
          '<div class="history-title">' + esc(it.title) + '</div>' +
          '<div class="history-meta">' +
            '<span class="history-tag">' + esc(it.platform) + '</span>' +
            '<span class="history-time">' + esc(it.time) + '</span>' +
          '</div>' +
        '</div>' +
        '<div class="history-actions">' +
          '<button class="history-btn re" data-act="re">重新解析</button>' +
          '<button class="history-btn" data-act="copy">复制</button>' +
          '<button class="history-btn del" data-act="del">删除</button>' +
        '</div>' +
      '</div>' +
      '<div class="history-detail"></div>';
    if (expandedId === it.id) {
      fillHistoryDetail(item, it.result, it.link);
    }
    list.appendChild(item);
  }
  document.getElementById('clearBtn').style.display = arr.length ? 'inline-block' : 'none';
}

// 事件委托：绑定一次到 historyList，避免循环闭包导致的“所有点击都指向最后一条”
document.addEventListener('DOMContentLoaded', function() {
  var list = document.getElementById('historyList');
  if (!list) return;
  list.addEventListener('click', function(e) {
    var btn = e.target.closest('button[data-act]');
    if (btn) {
      var act = btn.getAttribute('data-act');
      var item = btn.closest('.history-item');
      var id = item ? item.getAttribute('data-id') : '';
      var link = btn.closest('.history-info') ? '' : '';
      // 从当前渲染的 arr 里通过 id 找对应项（当前平台）
      var target = loadHistory(currentPlatform).find(function(h) { return h.id === id; });
      e.stopPropagation();
      if (act === 're') { if (target) reparse(target.link); }
      else if (act === 'copy') { if (target) copyText(target.link); }
      else if (act === 'del') { if (target) removeHistory(target.id); }
      return;
    }
    // 点击条目主体 -> 展开/收起
    var item = e.target.closest('.history-item');
    if (item) {
      var id = item.getAttribute('data-id');
      if (id) toggleExpand(id);
    }
  });
});

// 展开/收起：只保留一条展开（对齐桌面 expandedId）
function toggleExpand(id) {
  if (expandedId === id) {
    expandedId = null;
  } else {
    expandedId = id;
  }
  renderHistory();
}

// 就地填充本条 detail：先展示完整链接，再展示内容（视频=封面+播放 / 图片=直接展示）
function fillHistoryDetail(itemEl, result, link) {
  var detail = itemEl.querySelector('.history-detail');
  if (!detail) return;
  var linkBlock = link ? '<div class="detail-link">' + esc(link) + '</div>' : '';
  if (!result || !result.kind) { detail.innerHTML = linkBlock || ''; return; }
  if (result.kind === 'image') {
    detail.innerHTML = linkBlock + '<img src="' + mediaSrc(result.url) + '" onerror="this.alt=\'图片预览失败\'">';
  } else {
    detail.innerHTML = linkBlock +
      '<div class="hp-video" data-url="' + esc(result.url) + '" data-cover="' + esc(result.cover || '') + '">' +
      (result.cover ? '<img src="' + mediaSrc(result.cover) + '" onerror="this.alt=\'\'">' : '') +
      '<button class="hp-play" onclick="playPreview(this)">▶</button>' +
      '</div>';
  }
}

// 播放本条里的视频（保留封面作 poster；视频走本地 HTTP 代理 http://127.0.0.1:port）
function playPreview(btn) {
  var box = btn.parentNode;
  var url = box.getAttribute('data-url');
  var poster = box.getAttribute('data-cover') || '';
  if (!url) return;
  var p = poster ? ' poster="' + mediaSrc(poster) + '"' : '';
  box.innerHTML = '<video class="hp-video-el" src="' + proxyVideoSrc(url) + '"' + p + ' controls autoplay onerror="onVideoErr(this)"></video>';
}

// 视频加载失败：多为当前网络无法访问视频源（X 视频存于 video.twimg.com，国内网络常不可达）
function onVideoErr(el) {
  var box = el && el.parentNode;
  if (box) box.innerHTML = '<div style="padding:20px;text-align:center;color:#999;font-size:13px;line-height:1.7;">视频加载失败<br>当前网络可能无法访问该视频源<br>请检查网络或连接代理后重试</div>';
}

function addHistory(link, title, platform, itemId, result) {
  var key = platformKey(platform);             // 存到对应平台的桶
  var arr = loadHistory(key);
  // 判重依据：优先用作品唯一 id（aweme_id / tweet id），无 id 时回退到链接
  var dedupKey = (itemId && String(itemId).trim()) ? String(itemId).trim() : link;
  // 去重：同一作品 id 若已存在，先移除旧的，避免同一视频重复记录
  arr = arr.filter(function(h) { return h.itemId !== dedupKey; });
  var newId = 'h' + Date.now() + '_' + Math.floor(Math.random() * 10000);
  arr.unshift({
    id: newId,
    itemId: dedupKey,
    link: link,
    title: title,
    platform: platform,
    cover: result.cover || '',
    time: new Date().toLocaleString(),
    result: result
  });
  // 去重后若旧 expandedId 失配，同步到新 id
  if (expandedId && !arr.some(function(h) { return h.id === expandedId; })) expandedId = newId;
  if (arr.length > HISTORY_LIMIT) arr = arr.slice(0, HISTORY_LIMIT);
  saveHistory(key, arr);
  renderHistory();
}

function removeHistory(id) {
  var arr = loadHistory(currentPlatform).filter(function(h) { return h.id !== id; });
  saveHistory(currentPlatform, arr);
  renderHistory();
}

function clearHistory() {
  if (!window.confirm('确定清空当前平台的解析历史吗？此操作不可恢复。')) return;
  saveHistory(currentPlatform, []);
  renderHistory();
}

function reparse(link) {
  document.getElementById('input').value = link;
  handleParse();
}

function copyText(text) {
  if (window.DownloadBridge && window.DownloadBridge.copyText) {
    window.DownloadBridge.copyText(text);
  }
}

// 读取剪贴板并覆盖输入框（Android 端提供 readClipboard 返回文本）
function pasteFromClipboard() {
  if (window.DownloadBridge && window.DownloadBridge.readClipboard) {
    var txt = window.DownloadBridge.readClipboard();
    if (txt) {
      document.getElementById('input').value = txt;
      setStatus('已粘贴', 'success');
    } else {
      setStatus('剪贴板为空', 'error');
    }
  } else {
    setStatus('Android 粘贴功能未就绪', 'error');
  }
}

// 一键清除输入框并重置解析状态。
// 目的：规避 WebView 里 textarea 长时间选中/删除后输入框失灵的 bug（手动全选删除会卡死，
// 而 JS 直接改 value 的方式始终可靠，操作逻辑与粘贴按钮覆盖输入一致）。清除后重新聚焦输入框。
function clearInput() {
  var input = document.getElementById('input');
  input.value = '';
  parsing = false;                 // 释放解析锁，避免后续卡在"正在解析"
  videoUrl = '';
  currentCover = '';
  currentFiles = [];
  document.getElementById('result').classList.remove('show');
  document.getElementById('mediaStage').innerHTML = '';
  document.getElementById('downloadBtn').classList.remove('show');
  var status = document.getElementById('status');
  status.className = 'status';
  status.innerHTML = '';
  input.focus();
}

function esc(s) {
  return String(s == null ? '' : s)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
}

// 图片走安卓本地代理（带 UA/Referer 绕过 CDN 防盗链）。仅用于 <img> 的 src。
function mediaSrc(raw) {
  if (!raw) return '';
  return 'media://proxy?url=' + encodeURIComponent(raw);
}

// 视频走安卓本地 HTTP 代理（标准 http://127.0.0.1:port，WebView <video> 才能正常消费）。
// Android 端 StartProxyServer 转发时注入正确 UA/Referer，绕开 CDN 对 file:// Referer 的 403 拦截。
function proxyVideoSrc(raw) {
  if (!raw) return '';
  var port = (window.DownloadBridge && window.DownloadBridge.getProxyPort) ? window.DownloadBridge.getProxyPort() : 0;
  return 'http://127.0.0.1:' + port + '/media?url=' + encodeURIComponent(raw);
}

// ==================== 平台切换 ====================

var currentPlatform = 'douyin';  // 对齐桌面版 platform
var PLACEHOLDER = {
  douyin: '粘贴分享文本\n\n抖音：https://v.douyin.com/xxxxx/',
  x: '粘贴分享文本\n\nX：https://x.com/xxx/status/xxxx'
};

function switchPlatform(p) {
  currentPlatform = p;
  document.querySelectorAll('.platform-btn').forEach(function(b) {
    b.classList.toggle('active', b.getAttribute('data-platform') === p);
  });
  var input = document.getElementById('input');
  input.placeholder = PLACEHOLDER[p];
  // 切换平台：清空结果 + 重置展开 + 重新加载该平台历史
  document.getElementById('result').classList.remove('show');
  expandedId = null;
  renderHistory();
}

// ==================== 解析流程 ====================

function setStatus(text, type) {
  var el = document.getElementById('status');
  el.className = 'status' + (type ? ' ' + type : '');
  el.innerHTML = text;
}

var parsing = false;  // 解析锁：防止自动粘贴/手动点击重复触发

function handleParse() {
  var input = document.getElementById('input').value.trim();
  if (!input) { setStatus('请输入链接', 'error'); return; }
  if (!detectPlatform(input)) {
    setStatus('暂不支持该链接，支持：抖音 / X（Twitter）', 'error');
    return;
  }
  if (parsing) { setStatus('正在解析中，请稍候...', ''); return; }
  parsing = true;
  setStatus('<span class="spinner"></span>正在解析，请稍候...', '');
  document.getElementById('result').classList.remove('show');
  document.getElementById('mediaStage').innerHTML = '';
  if (window.DownloadBridge && window.DownloadBridge.processShareText) {
    window.DownloadBridge.processShareText(input);
  } else {
    parsing = false;
    setStatus('Android 桥接未就绪', 'error');
  }
}

function handleDownload() {
  if (!videoUrl) { setStatus('请先解析视频', 'error'); return; }
  var title = document.getElementById('title').textContent || 'video';
  if (window.DownloadBridge && window.DownloadBridge.downloadVideo) {
    window.DownloadBridge.downloadVideo(videoUrl, title);
  }
}

// 由 Android 端调用：更新状态
window.onStatusUpdate = function(msg, type) {
  setStatus(msg, type);
};

// 由 Android 端调用：视频/图片就绪
window.onVideoReady = function(title, url, cover, kind, itemId) {
  parsing = false;  // 释放解析锁
  videoUrl = url;
  currentCover = cover || '';
  currentKind = (kind === 'image') ? 'image' : 'video';
  currentFiles = [{ url: url, kind: currentKind, cover: currentCover }];
  document.getElementById('title').textContent = title;
  document.getElementById('mediaStage').innerHTML = buildStageHtml(currentKind, url, currentCover);
  document.getElementById('result').classList.add('show');
  document.getElementById('downloadBtn').classList.add('show');
  var input = document.getElementById('input').value.trim();
  var platform = detectPlatform(input) || '未知';
  addHistory(input, title, platform, itemId, {
    kind: currentKind,
    url: url,
    cover: currentCover
  });
};

// 渲染结果区：视频=封面卡片(点击播放)，图片=直接展示
function buildStageHtml(kind, url, cover) {
  if (kind === 'image') {
    return '<img class="stage-img" src="' + mediaSrc(url) + '" onerror="this.alt=\'图片加载失败\'">';
  }
  var poster = cover ? '<img class="stage-video-poster" src="' + mediaSrc(cover) + '" onerror="this.remove()">' : '';
  return '<div class="stage-video" data-url="' + esc(url) + '" data-cover="' + esc(cover || '') + '">' +
    poster +
    '<button class="stage-play" onclick="playStage(this)">▶</button>' +
    '</div>';
}

// 播放结果区视频（保留封面作 poster；视频走本地 HTTP 代理 http://127.0.0.1:port，WebView <video> 才能正常播放）
function playStage(btn) {
  var box = btn.parentNode;
  var url = box.getAttribute('data-url');
  var poster = box.getAttribute('data-cover') || '';
  if (!url) return;
  var p = poster ? ' poster="' + mediaSrc(poster) + '"' : '';
  box.innerHTML = '<video class="stage-video-el" src="' + proxyVideoSrc(url) + '"' + p + ' controls autoplay onerror="onVideoErr(this)"></video>';
}

// 自动检测粘贴：停止输入 800ms 后若含链接则自动解析（防抖，避免每字符触发）
var inputTimer = null;
document.getElementById('input').addEventListener('input', function() {
  clearTimeout(inputTimer);
  if (this.value.trim() && detectPlatform(this.value)) {
    inputTimer = setTimeout(handleParse, 800);
  }
});

// 初始化
switchPlatform('douyin');
renderHistory();
