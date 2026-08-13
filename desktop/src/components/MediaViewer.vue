<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { ArrowLeft, ArrowRight, Download, VideoPlay } from '@element-plus/icons-vue'
import { api, mediaProxyUrl, type ParseFile } from '../api/client'

const props = defineProps<{
  files: ParseFile[]
  title: string
  platform: string
  mediaType: string
  inputUrl?: string // 原始链接，用于"全部下载"（link 任务）
}>()

const index = ref(0)
const downloading = ref(false)
const downloadingAll = ref(false)
const livePlaying = ref(false)
const liveVideo = ref<HTMLVideoElement | null>(null)

const kindLabel: Record<string, string> = {
  video: '视频',
  image: '图片',
  gif: '动图',
  livephoto: 'Live图',
}

// 切换内容时重置 Live 图播放态（回到静态照片封面）
watch(index, () => {
  livePlaying.value = false
})

function prev() {
  index.value = (index.value - 1 + props.files.length) % props.files.length
}
function next() {
  index.value = (index.value + 1) % props.files.length
}

// Live 图卡片：点击后切到视频并自动播放，播完回到静态照片
async function playLive() {
  livePlaying.value = true
  await nextTick()
  try {
    await liveVideo.value?.play()
  } catch {
    /* 浏览器拦截/格式不支持时保留 controls 手动播放 */
  }
}

function downloadLabel(f: ParseFile): string {
  return f.kind === 'livephoto' ? 'Live图：照片+视频' : (kindLabel[f.kind] ?? f.kind)
}

// 媒体统一走本地代理：抖音/ X 的 CDN 都校验 Referer，代理按域名带正确 Referer 后才能播放/显示
function mediaSrc(raw?: string): string {
  if (!raw) return ''
  return mediaProxyUrl(raw)
}

// 解析完成后预加载全部媒体：图片/封面立即加载（切换秒开）；
// 视频本体不预载（避免一次下载全部视频），点击播放时再拉取
watch(
  () => props.files,
  (files) => {
    if (!files || !files.length) return
    for (const f of files) {
      if (f.kind === 'image' || f.kind === 'gif') {
        const img = new Image()
        img.src = mediaSrc(f.url)
      } else {
        const pic = f.image_url || f.cover
        if (pic) {
          const img = new Image()
          img.src = mediaSrc(pic)
        }
      }
    }
  },
  { immediate: true },
)

function pollTask(id: string, done: () => void) {
  const timer = setInterval(async () => {
    try {
      const task = await api.getTask(id)
      if (task.status === 'done') {
        clearInterval(timer)
        ElMessage.success(`下载完成：${task.output}`)
        done()
      } else if (task.status === 'failed' || task.status === 'cancelled') {
        clearInterval(timer)
        ElMessage.error(`下载失败：${task.error ?? task.status}`)
        done()
      }
    } catch {
      clearInterval(timer)
      ElMessage.error('查询任务状态失败')
      done()
    }
  }, 2000)
}

async function downloadCurrent() {
  const f = props.files[index.value]
  if (downloading.value) return
  downloading.value = true
  try {
    const task = await api.createTask('direct', f.url, {
      kind: f.kind,
      title: props.title,
      image_url: f.image_url,
    })
    ElMessage.success('任务已创建，开始下载…')
    pollTask(task.id, () => (downloading.value = false))
  } catch (e: any) {
    ElMessage.error(e?.response?.data?.detail ?? '创建任务失败')
    downloading.value = false
  }
}

async function downloadAll() {
  if (downloadingAll.value) return
  if (!props.inputUrl) {
    ElMessage.warning('缺少原始链接，无法全部下载')
    return
  }
  downloadingAll.value = true
  try {
    const task = await api.createTask('link', props.inputUrl, {
      remove_platform_wm: true,
    })
    ElMessage.success('任务已创建，开始下载全部内容…')
    pollTask(task.id, () => (downloadingAll.value = false))
  } catch (e: any) {
    ElMessage.error(e?.response?.data?.detail ?? '创建任务失败')
    downloadingAll.value = false
  }
}
</script>

<template>
  <div class="media-viewer">
    <div class="mv-header">
      <div class="mv-title">{{ title }}</div>
      <div class="mv-tags">
        <el-tag size="small">{{ platform }}</el-tag>
        <el-tag size="small" type="info">
          {{ mediaType === 'image' ? '图集' : '视频' }}（{{ files.length }} 个内容）
        </el-tag>
      </div>
    </div>

    <div class="mv-stage">
      <button
        v-if="files.length > 1"
        class="mv-arrow"
        :disabled="downloading || downloadingAll"
        @click="prev"
      >
        <el-icon><ArrowLeft /></el-icon>
      </button>

      <div class="mv-media">
        <img
          v-if="files[index].kind === 'image' || files[index].kind === 'gif'"
          :src="mediaSrc(files[index].url)"
          :alt="title"
          class="mv-img"
        />
        <!-- Live 图卡片：静态照片封面 + 点击播放视频，播完回到照片 -->
        <div v-else-if="files[index].kind === 'livephoto'" class="mv-live">
          <img
            v-if="!livePlaying"
            :src="mediaSrc(files[index].image_url || files[index].cover || '')"
            :alt="title"
            class="mv-live-photo"
          />
          <video
            v-show="livePlaying"
            ref="liveVideo"
            :src="mediaSrc(files[index].url)"
            controls
            autoplay
            class="mv-video"
            @ended="livePlaying = false"
          />
          <button v-if="!livePlaying" class="mv-live-play" title="点击播放 Live 图" @click="playLive">
            <el-icon :size="22"><VideoPlay /></el-icon>
          </button>
        </div>
        <video
          v-else
          :src="mediaSrc(files[index].url)"
          :poster="files[index].cover ? mediaSrc(files[index].cover) : undefined"
          controls
          preload="metadata"
          class="mv-video"
        />
      </div>

      <button
        v-if="files.length > 1"
        class="mv-arrow"
        :disabled="downloading || downloadingAll"
        @click="next"
      >
        <el-icon><ArrowRight /></el-icon>
      </button>
    </div>

    <div v-if="files.length > 1" class="mv-indicator">
      {{ index + 1 }} / {{ files.length }}
    </div>

    <div class="mv-actions">
      <el-button
        type="primary"
        :icon="Download"
        :loading="downloading"
        @click="downloadCurrent"
      >
        下载当前（{{ downloadLabel(files[index]) }}）
      </el-button>
      <el-button
        v-if="files.length > 1"
        type="success"
        :icon="Download"
        :loading="downloadingAll"
        @click="downloadAll"
      >
        全部下载
      </el-button>
    </div>
  </div>
</template>

<style scoped>
.media-viewer { width: 100%; }
.mv-header { display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; margin-bottom: 12px; }
.mv-title { font-size: 15px; font-weight: 500; word-break: break-all; }
.mv-tags { display: flex; gap: 6px; flex-shrink: 0; }
.mv-stage { display: flex; align-items: center; gap: 12px; }
.mv-arrow {
  width: 36px; height: 36px; border-radius: 50%;
  border: 1px solid #dcdfe6; background: #fff; cursor: pointer;
  display: flex; align-items: center; justify-content: center; color: #606266;
  flex-shrink: 0;
}
.mv-arrow:hover:not(:disabled) { color: #409eff; border-color: #409eff; }
.mv-arrow:disabled { opacity: 0.4; cursor: not-allowed; }
.mv-media { flex: 1; min-width: 0; background: #f5f7fa; border-radius: 8px; overflow: hidden; display: flex; justify-content: center; }
.mv-img { max-width: 100%; max-height: 420px; object-fit: contain; display: block; }
.mv-video { width: 100%; max-height: 420px; display: block; }
.mv-live { position: relative; width: 100%; max-height: 420px; display: flex; justify-content: center; align-items: center; }
.mv-live-photo { max-width: 100%; max-height: 420px; object-fit: contain; display: block; }
.mv-live-play {
  position: absolute; inset: 0; margin: auto; width: 56px; height: 56px;
  border-radius: 50%; border: none; cursor: pointer; color: #fff;
  background: rgba(0, 0, 0, 0.55); display: flex; align-items: center; justify-content: center;
  transition: background 0.2s, transform 0.1s;
}
.mv-live-play:hover { background: rgba(64, 158, 255, 0.85); }
.mv-live-play:active { transform: scale(0.94); }
.mv-indicator { text-align: center; color: #909399; font-size: 12px; margin-top: 8px; }
.mv-actions { margin-top: 12px; display: flex; gap: 8px; }
</style>
