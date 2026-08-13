//! Bug 反馈邮件发送（SMTP），逻辑与 Python 版 backend/app/notify.py 一致。
//! 成功返回 Ok(())，失败返回 Err(错误信息字符串)。

use std::time::Duration;

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Message, SmtpTransport, Transport};

use crate::config::Config;

const SUBJECT: &str = "【水印工坊】Bug 反馈";

pub fn send_feedback_email(cfg: &Config, content: &str, contact: Option<&str>) -> Result<(), String> {
    let user = cfg.smtp_user.trim();
    let auth = cfg.smtp_auth_code.trim();
    let to = cfg.feedback_to.trim();
    if user.is_empty() || auth.is_empty() || to.is_empty() {
        return Err("开发者未配置反馈邮箱，暂时无法接收反馈".to_string());
    }
    let body = format!(
        "反馈内容：\n{}\n\n联系方式：{}",
        content,
        contact.unwrap_or("未填写")
    );
    let email = Message::builder()
        .from(user.parse().map_err(|_| "发件邮箱格式错误".to_string())?)
        .to(to.parse().map_err(|_| "收件邮箱格式错误".to_string())?)
        .subject(SUBJECT)
        .header(ContentType::TEXT_PLAIN)
        .body(body)
        .map_err(|e| format!("邮件构造失败：{e}"))?;

    let host = if cfg.smtp_host.trim().is_empty() {
        "smtp.qq.com"
    } else {
        cfg.smtp_host.trim()
    };
    let port = if cfg.smtp_port == 0 { 465 } else { cfg.smtp_port };
    let tls_params = TlsParameters::new(host.to_string())
        .map_err(|e| format!("TLS 参数初始化失败：{e}"))?;
    let mailer = SmtpTransport::builder_dangerous(host.to_string())
        .port(port)
        .tls(Tls::Wrapper(tls_params))
        .credentials(Credentials::new(user.to_string(), auth.to_string()))
        .timeout(Some(Duration::from_secs(20)))
        .build();

    match mailer.send(&email) {
        Ok(_) => Ok(()),
        Err(e) => {
            // 认证失败单独提示（对应 Python SMTPAuthenticationError）
            if format!("{e}").contains("authentication") || format!("{e}").contains("535") {
                Err("SMTP 认证失败（请检查邮箱账号与授权码）".to_string())
            } else {
                Err(format!("邮件发送失败：{e}"))
            }
        }
    }
}
