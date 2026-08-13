import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// 开发阶段由 Vite 代理到本地后端，避免 CORS 与硬编码端口
export default defineConfig({
  plugins: [vue()],
  server: {
    port: 5173,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:17890',
        changeOrigin: true,
      },
    },
  },
})
