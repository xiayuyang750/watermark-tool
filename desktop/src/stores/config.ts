import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '../api/client'

export const useConfigStore = defineStore('config', () => {
  const removePlatformWm = ref(true) // 开关1：去平台水印
  const removeContentWm = ref(false) // 开关2：去内容水印（M2 开发中）
  const outputDir = ref('')
  const smtpHost = ref('')
  const smtpPort = ref(465)
  const smtpUser = ref('')
  const smtpAuthCode = ref('') // 授权码不回显，仅保存时提交
  const feedbackTo = ref('')
  const loaded = ref(false)

  async function fetchConfig() {
    const cfg = await api.getConfig()
    removePlatformWm.value = Boolean(cfg.remove_platform_wm)
    removeContentWm.value = Boolean(cfg.remove_content_wm)
    outputDir.value = String(cfg.output_dir ?? '')
    smtpHost.value = String(cfg.smtp_host ?? '')
    smtpPort.value = Number(cfg.smtp_port ?? 465)
    smtpUser.value = String(cfg.smtp_user ?? '')
    feedbackTo.value = String(cfg.feedback_to ?? '')
    loaded.value = true
  }

  async function save() {
    const body: Record<string, unknown> = {
      remove_platform_wm: removePlatformWm.value,
      remove_content_wm: removeContentWm.value,
      output_dir: outputDir.value,
      smtp_host: smtpHost.value,
      smtp_port: smtpPort.value,
      smtp_user: smtpUser.value,
      feedback_to: feedbackTo.value,
    }
    // 授权码仅在用户重新填写时覆盖（后端不回显该字段，空值不动它）
    if (smtpAuthCode.value) {
      body.smtp_auth_code = smtpAuthCode.value
    }
    await api.saveConfig(body)
  }

  return {
    removePlatformWm,
    removeContentWm,
    outputDir,
    smtpHost,
    smtpPort,
    smtpUser,
    smtpAuthCode,
    feedbackTo,
    loaded,
    fetchConfig,
    save,
  }
})
