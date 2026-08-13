<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ElMessage } from 'element-plus'
import { ChatDotRound, Setting } from '@element-plus/icons-vue'
import { api, type GitHubRelease } from './api/client'
import { useConfigStore } from './stores/config'

// GitHub 更新源（发布后填入仓库 owner/repo，例如 'yourname' / 'watermark-tool'）
const GITHUB_OWNER = 'xiayuyang750'
const GITHUB_REPO = 'watermark-tool'

const config = useConfigStore()
const drawerOpen = ref(false)
const feedbackOpen = ref(false)
const feedbackContent = ref('')
const feedbackContact = ref('')
const submitting = ref(false)

// 版本与更新检查
const currentVersion = ref('')
const checkingUpdate = ref(false)
const updateStatus = ref<'idle' | 'latest' | 'outdated' | 'error'>('idle')
const latestVersion = ref('')
const releaseUrl = ref('')

// X 平台登录引导状态
const xLoginStatus = ref('idle') // idle / running / done / error
const xLoginError = ref('')
const xLoginRunning = ref(false)
let xPollTimer: number | undefined

onMounted(() => {
  if (!config.loaded) config.fetchConfig()
  // 读取后端实际版本号用于展示与更新对比
  api
    .health()
    .then((h: any) => {
      currentVersion.value = h?.version ?? ''
    })
    .catch(() => {})
})

/** 语义化版本比较：a>b 返回 1，a<b 返回 -1，相等返回 0。 */
function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map((n) => parseInt(n, 10) || 0)
  const pb = b.split('.').map((n) => parseInt(n, 10) || 0)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = pa[i] ?? 0
    const y = pb[i] ?? 0
    if (x !== y) return x > y ? 1 : -1
  }
  return 0
}

async function checkUpdate() {
  if (!GITHUB_OWNER || !GITHUB_REPO) {
    updateStatus.value = 'error'
    ElMessage.warning('更新源未配置（缺少 GitHub 仓库信息）')
    return
  }
  checkingUpdate.value = true
  updateStatus.value = 'idle'
  try {
    const r: GitHubRelease = await api.checkUpdate(GITHUB_OWNER, GITHUB_REPO)
    latestVersion.value = (r.tag_name || '').replace(/^v/i, '')
    releaseUrl.value = r.html_url || ''
    const cur = currentVersion.value.replace(/^v/i, '')
    if (!cur) throw new Error('无法获取当前版本')
    if (compareVersions(latestVersion.value, cur) > 0) {
      updateStatus.value = 'outdated'
      ElMessage.info(`发现新版本 v${latestVersion.value}，可前往下载页更新`)
    } else {
      updateStatus.value = 'latest'
      ElMessage.success('已是最新版本')
    }
  } catch {
    updateStatus.value = 'error'
    ElMessage.error('检查更新失败（网络异常或仓库不可访问）')
  } finally {
    checkingUpdate.value = false
  }
}

async function startXLogin() {
  xLoginRunning.value = true
  xLoginStatus.value = 'running'
  xLoginError.value = ''
  try {
    await api.startXLogin()
    ElMessage.info('请在弹出的 Edge 窗口中完成 X 登录（支持扫码或账号密码）')
    xPollTimer = window.setInterval(async () => {
      try {
        const s = await api.xLoginStatus()
        xLoginStatus.value = s.status
        xLoginError.value = s.error ?? ''
        if (s.status === 'done' || s.status === 'error') {
          xLoginRunning.value = false
          if (xPollTimer) {
            window.clearInterval(xPollTimer)
            xPollTimer = undefined
          }
          if (s.status === 'done') {
            ElMessage.success('X 登录成功，登录墙推文现在可以解析了')
          } else {
            ElMessage.error(`X 登录失败：${s.error ?? '未知错误'}`)
          }
        }
      } catch {
        /* 忽略瞬时网络错误，继续轮询 */
      }
    }, 2000)
  } catch (e: any) {
    xLoginStatus.value = 'error'
    xLoginError.value = e?.response?.data?.detail ?? '启动 X 登录失败'
    xLoginRunning.value = false
  }
}

function onSwitch1Changed() {
  config.save().catch(() => ElMessage.error('保存设置失败'))
}

async function saveOutputDir() {
  try {
    await config.save()
    ElMessage.success('设置已保存')
  } catch {
    ElMessage.error('保存失败')
  }
}

async function submitFeedback() {
  if (!feedbackContent.value.trim()) {
    ElMessage.warning('请填写反馈内容')
    return
  }
  submitting.value = true
  try {
    await api.feedback(feedbackContent.value.trim(), feedbackContact.value.trim())
    ElMessage.success('反馈已发送，感谢你的反馈！')
    feedbackContent.value = ''
    feedbackContact.value = ''
    feedbackOpen.value = false
  } catch (e: any) {
    ElMessage.error(e?.response?.data?.detail ?? '反馈发送失败')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="layout">
    <header class="topbar">
      <div class="logo">水印工坊</div>
      <div class="top-actions">
        <span class="disclaimer">仅限个人合法素材使用</span>
        <el-tooltip content="设置与反馈" placement="bottom">
          <el-button :icon="Setting" circle class="settings-btn" @click="drawerOpen = true" />
        </el-tooltip>
      </div>
    </header>
    <main class="main">
      <router-view />
    </main>

    <el-drawer v-model="drawerOpen" title="设置" size="320px">
      <el-form label-width="90px">
        <el-form-item label="输出目录">
          <el-input v-model="config.outputDir" placeholder="下载保存路径（不带引号）" />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" @click="saveOutputDir">保存</el-button>
        </el-form-item>
        <el-divider />
        <el-form-item label="去平台水印">
          <el-switch v-model="config.removePlatformWm" @change="onSwitch1Changed" />
        </el-form-item>
        <el-form-item label="去内容水印">
          <el-tooltip content="功能开发中，敬请期待" placement="top">
            <span><el-switch v-model="config.removeContentWm" disabled /></span>
          </el-tooltip>
        </el-form-item>
        <el-divider />
        <el-form-item label="X 平台登录">
          <div class="x-login">
            <el-button
              type="primary"
              :loading="xLoginRunning"
              @click="startXLogin"
            >
              打开 X 登录
            </el-button>
            <div class="x-login-tip">
              <span v-if="xLoginStatus === 'done'" class="ok">已登录：登录墙/私密推文可解析</span>
              <span v-else-if="xLoginStatus === 'error'" class="err">登录失败：{{ xLoginError }}</span>
              <span v-else-if="xLoginRunning" class="wait">请在弹出的浏览器窗口中完成登录…</span>
              <span v-else class="idle">可选：用于解析需登录才能看的推文</span>
            </div>
          </div>
        </el-form-item>
        <el-divider />
        <el-form-item label="问题反馈">
          <el-button :icon="ChatDotRound" @click="feedbackOpen = true">反馈 Bug / 建议</el-button>
        </el-form-item>
        <el-divider />
        <el-form-item label="关于">
          <div class="about">
            <div class="ver-line">
              当前版本：<span class="ver-num">v{{ currentVersion || '--' }}</span>
            </div>
            <el-button :loading="checkingUpdate" @click="checkUpdate">检查更新</el-button>
            <div class="update-tip">
              <span v-if="updateStatus === 'latest'" class="ok">已是最新版本</span>
              <span v-else-if="updateStatus === 'outdated'" class="err">
                发现新版本 v{{ latestVersion }}：
                <a :href="releaseUrl" target="_blank" rel="noopener">前往下载页</a>
              </span>
              <span v-else-if="updateStatus === 'error'" class="err">检查更新失败，请稍后重试</span>
              <span v-else class="idle">从 GitHub Releases 检查新版本</span>
            </div>
          </div>
        </el-form-item>
      </el-form>
    </el-drawer>

    <el-dialog v-model="feedbackOpen" title="问题反馈" width="480px">
      <el-input
        v-model="feedbackContent"
        type="textarea"
        :rows="5"
        placeholder="请描述你遇到的问题或建议（例如：解析失败、下载报错、想要的功能…）"
      />
      <el-input
        v-model="feedbackContact"
        placeholder="联系方式（选填，便于开发者回复）"
        class="fb-contact"
      />
      <template #footer>
        <el-button @click="feedbackOpen = false">取消</el-button>
        <el-button type="primary" :loading="submitting" @click="submitFeedback">提交反馈</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body, #app { height: 100%; }
.layout { height: 100%; display: flex; flex-direction: column; background: #f5f7fa; }
.topbar {
  height: 56px; background: #1f2d3d; color: #fff; flex-shrink: 0;
  display: flex; align-items: center; justify-content: space-between; padding: 0 20px;
}
.logo { font-size: 17px; font-weight: 600; }
.top-actions { display: flex; align-items: center; gap: 12px; }
.disclaimer { font-size: 12px; color: #7f8fa4; }
.settings-btn { color: #1f2d3d; }
.main { flex: 1; overflow-y: auto; padding: 20px; display: flex; justify-content: center; }
.fb-contact { margin-top: 10px; }
.x-login { width: 100%; }
.x-login-tip { margin-top: 8px; font-size: 12px; line-height: 1.5; }
.x-login-tip .ok { color: #67c23a; }
.x-login-tip .err { color: #f56c6c; }
.x-login-tip .wait { color: #e6a23c; }
.x-login-tip .idle { color: #909399; }
.about { width: 100%; }
.ver-line { margin-bottom: 10px; font-size: 13px; }
.ver-num { font-weight: 600; color: #1f2d3d; }
.update-tip { margin-top: 8px; font-size: 12px; line-height: 1.5; }
.update-tip .ok { color: #67c23a; }
.update-tip .err { color: #f56c6c; }
.update-tip .idle { color: #909399; }
.update-tip a { color: #409eff; }
</style>
