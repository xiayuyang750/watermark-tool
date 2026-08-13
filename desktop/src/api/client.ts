import axios from 'axios'

export interface ParseFile {
  kind: string // video / image / gif / livephoto
  url: string
  label?: string
  cover?: string // 封面/预览图（视频与 Live 图可选）
  image_url?: string // Live 图的静态照片直链（与 url 视频组成原生 Live 图）
}

export interface ParseResult {
  platform: string
  title: string
  media_type: string
  files: ParseFile[]
}

export type Platform = 'douyin' | 'x'

export interface XLoginStatus {
  ok: boolean
  status: string // idle / running / done / error
  error?: string
}

export interface TaskItem {
  id: string
  type: string
  status: string // pending / running / done / failed / cancelled
  progress: number
  output: string | null
  error: string | null
  created_at: string
}

/** GitHub Releases latest 响应（检查更新用，仅取需要的字段）。 */
export interface GitHubRelease {
  tag_name: string
  html_url: string
  assets: { name: string; browser_download_url: string }[]
}

// 开发环境走 vite 代理（同源）；打包后（Tauri 生产模式）直连本地后端
const baseURL = import.meta.env.DEV ? '/api/v1' : 'http://127.0.0.1:17890/api/v1'
const client = axios.create({ baseURL, timeout: 60000 })

/** 把素材直链转成本地媒体代理地址（绕过 CDN 防盗链，支持直接播放/显示）。 */
export function mediaProxyUrl(raw: string): string {
  const origin = import.meta.env.DEV ? '' : 'http://127.0.0.1:17890'
  return `${origin}/api/v1/media?url=${encodeURIComponent(raw)}`
}

export const api = {
  health: () => client.get('/health').then((r) => r.data),
  parse: (url: string, removePlatformWm: boolean) =>
    client.post<ParseResult>('/parse', { url, remove_platform_wm: removePlatformWm }).then((r) => r.data),
  createTask: (type: 'link' | 'direct', url: string, options: Record<string, unknown>) =>
    client.post<TaskItem>('/tasks', { type, url, options }).then((r) => r.data),
  getTask: (id: string) => client.get<TaskItem>(`/tasks/${id}`).then((r) => r.data),
  listTasks: () => client.get<TaskItem[]>('/tasks').then((r) => r.data),
  cancelTask: (id: string) => client.post(`/tasks/${id}/cancel`).then((r) => r.data),
  getConfig: () => client.get<Record<string, unknown>>('/config').then((r) => r.data),
  saveConfig: (cfg: Record<string, unknown>) => client.put('/config', cfg).then((r) => r.data),
  feedback: (content: string, contact: string) =>
    client.post('/feedback', { content, contact }).then((r) => r.data),
  startXLogin: () => client.post('/x/login/start').then((r) => r.data),
  xLoginStatus: () => client.get<XLoginStatus>('/x/login/status').then((r) => r.data),
  // 检查更新：直连 GitHub Releases API（GitHub 允许跨域，无需后端中转）
  checkUpdate: (owner: string, repo: string) =>
    axios
      .get<GitHubRelease>(`https://api.github.com/repos/${owner}/${repo}/releases/latest`, { timeout: 15000 })
      .then((r) => r.data),
}
