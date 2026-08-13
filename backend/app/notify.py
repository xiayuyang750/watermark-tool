"""Bug 反馈邮件发送（SMTP）。"""
import smtplib
from email.header import Header
from email.mime.text import MIMEText

SUBJECT = "【水印工坊】Bug 反馈"


def send_feedback_email(cfg: dict, content: str, contact: str | None) -> str:
    """发送反馈邮件，成功返回 None，失败返回错误信息。"""
    user = cfg.get("smtp_user") or ""
    auth = cfg.get("smtp_auth_code") or ""
    to = cfg.get("feedback_to") or ""
    if not (user and auth and to):
        return "开发者未配置反馈邮箱，暂时无法接收反馈"
    body = f"反馈内容：\n{content}\n\n联系方式：{contact or '未填写'}"
    msg = MIMEText(body, "plain", "utf-8")
    msg["From"] = user
    msg["To"] = to
    msg["Subject"] = Header(SUBJECT, "utf-8")
    try:
        with smtplib.SMTP_SSL(cfg.get("smtp_host") or "smtp.qq.com", int(cfg.get("smtp_port") or 465), timeout=20) as s:
            s.login(user, auth)
            s.sendmail(user, [to], msg.as_string())
    except smtplib.SMTPAuthenticationError:
        return "SMTP 认证失败（请检查邮箱账号与授权码）"
    except Exception as exc:
        return f"邮件发送失败：{exc}"
    return ""
