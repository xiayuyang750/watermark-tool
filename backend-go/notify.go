// Bug 反馈邮件发送（SMTP），逻辑与 backend-rust/src/notify.rs 一致。
// 支持 465 隐式 TLS（SMTPS）与 587 STARTTLS，零第三方依赖。

package main

import (
	"crypto/tls"
	"fmt"
	"net"
	"net/smtp"
	"strconv"
	"strings"
	"time"
)

const feedbackSubject = "【水印工坊】Bug 反馈"

// sendFeedbackEmail 发送反馈邮件；成功返回 nil，失败返回错误。
func sendFeedbackEmail(cfg *Config, content, contact string) error {
	user := strings.TrimSpace(cfg.SMTPUser)
	auth := strings.TrimSpace(cfg.SMTPAuthCode)
	to := strings.TrimSpace(cfg.FeedbackTo)
	if user == "" || auth == "" || to == "" {
		return fmt.Errorf("开发者未配置反馈邮箱，暂时无法接收反馈")
	}
	host := strings.TrimSpace(cfg.SMTPHost)
	if host == "" {
		host = "smtp.qq.com"
	}
	port := cfg.SMTPPort
	if port == 0 {
		port = 465
	}
	body := fmt.Sprintf("反馈内容：\n%s\n\n联系方式：%s", content, orDefault(contact, "未填写"))

	// 构造 RFC 5322 邮件文本（含 From/To/Subject/Content-Type 头）
	msg := strings.Join([]string{
		"From: " + user,
		"To: " + to,
		"Subject: " + feedbackSubject,
		"MIME-Version: 1.0",
		"Content-Type: text/plain; charset=UTF-8",
		"",
		body,
	}, "\r\n")

	addr := net.JoinHostPort(host, strconv.Itoa(port))
	var c *smtp.Client
	var conn net.Conn
	var err error
	if port == 465 {
		// 隐式 TLS（SMTPS）
		conn, err = tls.Dial("tcp", addr, &tls.Config{ServerName: host})
		if err != nil {
			return fmt.Errorf("邮件发送失败：%v", err)
		}
		c, err = smtp.NewClient(conn, host)
	} else {
		// 明文连接后 STARTTLS（587）
		conn, err = net.DialTimeout("tcp", addr, 20*time.Second)
		if err != nil {
			return fmt.Errorf("邮件发送失败：%v", err)
		}
		c, err = smtp.NewClient(conn, host)
		if err == nil {
			err = c.StartTLS(&tls.Config{ServerName: host})
		}
	}
	if err != nil {
		conn.Close()
		return fmt.Errorf("邮件发送失败：%v", err)
	}
	defer c.Close()

	if err := c.Auth(smtp.PlainAuth("", user, auth, host)); err != nil {
		// 认证失败单独提示（对应 Python SMTPAuthenticationError）
		if strings.Contains(err.Error(), "535") || strings.Contains(err.Error(), "authentication") {
			return fmt.Errorf("SMTP 认证失败（请检查邮箱账号与授权码）")
		}
		return fmt.Errorf("邮件发送失败：%v", err)
	}
	if err := c.Mail(user); err != nil {
		return fmt.Errorf("邮件发送失败：%v", err)
	}
	if err := c.Rcpt(to); err != nil {
		return fmt.Errorf("邮件发送失败：%v", err)
	}
	w, err := c.Data()
	if err != nil {
		return fmt.Errorf("邮件发送失败：%v", err)
	}
	if _, err := w.Write([]byte(msg)); err != nil {
		w.Close()
		return fmt.Errorf("邮件发送失败：%v", err)
	}
	if err := w.Close(); err != nil {
		return fmt.Errorf("邮件发送失败：%v", err)
	}
	return c.Quit()
}

func orDefault(s, def string) string {
	if strings.TrimSpace(s) == "" {
		return def
	}
	return s
}
