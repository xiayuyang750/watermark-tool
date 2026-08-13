import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '../api/client'

export const useConfigStore = defineStore('config', () => {
  const removePlatformWm = ref(true) // 开关1：去平台水印
  const removeContentWm = ref(false) // 开关2：去内容水印（M2 开发中）
  const outputDir = ref('')
  const loaded = ref(false)

  async function fetchConfig() {
    const cfg = await api.getConfig()
    removePlatformWm.value = Boolean(cfg.remove_platform_wm)
    removeContentWm.value = Boolean(cfg.remove_content_wm)
    outputDir.value = String(cfg.output_dir ?? '')
    loaded.value = true
  }

  async function save() {
    await api.saveConfig({
      remove_platform_wm: removePlatformWm.value,
      remove_content_wm: removeContentWm.value,
      output_dir: outputDir.value,
    })
  }

  return { removePlatformWm, removeContentWm, outputDir, loaded, fetchConfig, save }
})
