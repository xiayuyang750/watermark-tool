<script setup lang="ts">
import { ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Delete, DocumentCopy, Link, VideoPlay } from '@element-plus/icons-vue'
import { api, mediaProxyUrl, type HistoryItem, type ParseResult, type Platform } from '../api/client'
import { useConfigStore } from '../stores/config'
import MediaViewer from '../components/MediaViewer.vue'

const config = useConfigStore()

const platform = ref<Platform>('douyin')

const url = ref('')
const parsing = ref(false)
const result = ref<ParseResult | null>(null)
const history = ref<HistoryItem[]>([])
const expandedId = ref<string | null>(null)

// 从后端加载当前平台历史
async function loadHistory() {
  try {
    const data = await api.listHistory(platform.value)
    history.value = data.items ?? []
  } catch {
    history.value = []
  }
}

// 全量保存当前平台历史到后端（清空/删除后整体提交）
async function saveHistory(list: HistoryItem[]) {
  try {
    const data = await api.replaceHistory(platform.value, list)
    history.value = data.items ?? list
  } catch {
    /* 保存失败保持现状 */
  }
}

// 切换平台：重置解析结果并加载对应平台的历史（不会打断正在进行的解析请求）
watch(
  platform,
  async () => {
    result.value = null
    url.value = ''
    expandedId.value = null
    await loadHistory()
  },
  { immediate: true },
)

const placeholderText = {
  douyin: '粘贴或输入抖音分享链接（可直接编辑修改）',
  x: '粘贴或输入 X/Twitter 推文链接（如 x.com/用户/status/推文ID）',
}

async function pasteFromClipboard() {
  try {
    const text = await navigator.clipboard.readText()
    if (text) url.value = text.trim()
  } catch {
    ElMessage.warning('无法读取剪贴板（无权限或浏览器限制），请手动粘贴')
  }
}

async function onParse() {
  const input = url.value.trim()
  if (!input) {
    ElMessage.warning(platform.value === 'douyin' ? '请先输入或粘贴抖音分享链接' : '请先输入或粘贴 X 推文链接')
    return
  }
  // 快照发起平台：切换平台不会打断本请求，结果始终存入发起平台的历史
  const fromPlatform = platform.value
  parsing.value = true
  result.value = null
  try {
    const res = await api.parse(input, config.removePlatformWm)
    const item: HistoryItem = {
      id: Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
      input,
      time: new Date().toLocaleString(),
      result: res,
    }
    try {
      await api.addHistory(fromPlatform, item)
    } catch {
      /* 历史保存失败不影响解析结果 */
    }
    // 仅当仍停留在发起平台时展示结果；否则提示已入历史
    if (platform.value === fromPlatform) {
      result.value = res
      expandedId.value = null
    } else {
      ElMessage.success(`${fromPlatform === 'douyin' ? '抖音' : 'X'}解析完成，已存入${fromPlatform === 'douyin' ? '抖音' : 'X'}解析历史`)
    }
  } catch (e: any) {
    ElMessage.error(e?.response?.data?.detail ?? '解析失败，请稍后重试')
  } finally {
    parsing.value = false
  }
}

function toggleExpand(id: string) {
  expandedId.value = expandedId.value === id ? null : id
}

// 复制历史原始链接
function copyLink(text: string) {
  navigator.clipboard
    .writeText(text)
    .then(() => ElMessage.success('链接已复制'))
    .catch(() => ElMessage.warning('复制失败，请手动选择复制'))
}

// 用历史原始链接重新解析（链接可能已过期，重新解析获取最新直链）
async function reparse(input: string) {
  url.value = input
  await onParse()
}

async function clearHistory() {
  try {
    await ElMessageBox.confirm('确定清空全部解析历史吗？此操作不可恢复。', '清空历史', {
      type: 'warning',
      confirmButtonText: '清空',
      cancelButtonText: '取消',
    })
  } catch {
    return // 用户取消
  }
  history.value = []
  await saveHistory([])
  ElMessage.success('历史已清空')
}

// 单独删除某一条历史记录
async function removeHistory(id: string) {
  history.value = history.value.filter((h) => h.id !== id)
  await saveHistory(history.value)
  if (expandedId.value === id) expandedId.value = null
  ElMessage.success('已删除该条记录')
}

// 历史缩略图：统一走本地代理（CDN 校验 Referer，代理按域名带正确来源）
function thumbUrl(item: HistoryItem): string | null {
  const f = item.result.files?.[0]
  if (!f) return null
  if (f.kind === 'image' || f.kind === 'gif') return mediaProxyUrl(f.url)
  const pic = f.image_url || f.cover
  return pic ? mediaProxyUrl(pic) : null
}
</script>

<template>
  <div class="home">
    <div class="platform-switch">
      <el-radio-group v-model="platform" size="large">
        <el-radio-button value="douyin">抖音</el-radio-button>
        <el-radio-button value="x">X（推特）</el-radio-button>
      </el-radio-group>
    </div>

    <el-card class="card">
      <template #header><span>解析作品链接</span></template>
      <el-input
        v-model="url"
        :placeholder="placeholderText[platform]"
        clearable
        @keyup.enter="onParse"
      >
        <template #append>
          <el-tooltip content="从剪贴板读取链接" placement="top">
            <el-button :icon="DocumentCopy" @click="pasteFromClipboard" />
          </el-tooltip>
        </template>
      </el-input>
      <el-button type="primary" :icon="Link" :loading="parsing" class="parse-btn" @click="onParse">
        解析链接
      </el-button>
    </el-card>

    <el-card v-if="result" class="card">
      <template #header><span>解析结果</span></template>
      <MediaViewer
        :files="result.files"
        :title="result.title"
        :platform="platform"
        :media-type="result.media_type"
        :input-url="url.trim()"
      />
    </el-card>

    <el-card class="card">
      <template #header>
        <div class="hist-header">
          <span>解析历史</span>
          <el-button v-if="history.length" size="small" text type="danger" @click="clearHistory">
            清空
          </el-button>
        </div>
      </template>
      <el-empty v-if="!history.length" description="暂无解析历史" :image-size="60" />
      <div v-else class="hist-list">
        <div
          v-for="item in history"
          :key="item.id"
          class="hist-item"
          :class="{ expanded: expandedId === item.id }"
          @click="toggleExpand(item.id)"
        >
          <div class="hist-head">
            <div class="hist-thumb">
              <img v-if="thumbUrl(item)" :src="thumbUrl(item)" alt="" />
              <el-icon v-else class="hist-thumb-icon"><VideoPlay /></el-icon>
            </div>
            <div class="hist-info">
              <div class="hist-title">{{ item.result.title }}</div>
              <div class="hist-link" @click.stop>
                <el-icon :size="12"><Link /></el-icon>
                <span class="hist-link-text" :title="item.input">{{ item.input }}</span>
                <el-button size="small" text type="primary" @click="copyLink(item.input)">复制</el-button>
                <el-button size="small" text type="primary" @click="reparse(item.input)">重新解析</el-button>
              </div>
              <div class="hist-meta">
                <el-tag size="small">{{ item.result.platform }}</el-tag>
                <el-tag size="small" type="info">
                  {{ item.result.media_type === 'image' ? '图集' : '视频' }}
                </el-tag>
                <span class="hist-time">{{ item.time }}</span>
              </div>
            </div>
            <el-button class="hist-del" size="small" text type="danger" :icon="Delete" @click.stop="removeHistory(item.id)" />
          </div>
          <div v-if="expandedId === item.id" class="hist-detail" @click.stop>
            <MediaViewer
              :files="item.result.files"
              :title="item.result.title"
              :platform="platform"
              :media-type="item.result.media_type"
              :input-url="item.input"
            />
          </div>
        </div>
      </div>
    </el-card>
  </div>
</template>

<style scoped>
.home { max-width: 720px; width: 100%; min-width: 0; }
.card { margin-bottom: 16px; }
.platform-switch { display: flex; justify-content: center; margin-bottom: 16px; }
.parse-btn { margin-top: 12px; }
.hist-header { display: flex; justify-content: space-between; align-items: center; }
.hist-list { display: flex; flex-direction: column; gap: 8px; }
.hist-item { border: 1px solid #e4e7ed; border-radius: 8px; padding: 10px; cursor: pointer; transition: border-color 0.2s; }
.hist-item:hover { border-color: #409eff; }
.hist-item.expanded { border-color: #409eff; }
.hist-head { display: flex; align-items: center; gap: 12px; }
.hist-thumb {
  width: 64px; height: 64px; border-radius: 6px; background: #f5f7fa; flex-shrink: 0;
  display: flex; align-items: center; justify-content: center; overflow: hidden;
}
.hist-thumb img { width: 100%; height: 100%; object-fit: cover; }
.hist-thumb-icon { font-size: 26px; color: #a8abb2; }
.hist-info { min-width: 0; flex: 1; }
.hist-title { font-size: 14px; margin-bottom: 6px; word-break: break-all; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.hist-link { display: flex; align-items: center; gap: 4px; font-size: 12px; color: #909399; margin-bottom: 4px; min-width: 0; }
.hist-link-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.hist-meta { display: flex; align-items: center; gap: 6px; }
.hist-time { font-size: 12px; color: #909399; }
.hist-detail { margin-top: 10px; border-top: 1px dashed #e4e7ed; padding-top: 10px; }
</style>
