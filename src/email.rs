use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::config::EmailProvider;

#[derive(Clone)]
pub struct Mailer {
    provider: EmailProvider,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrencyTotal {
    pub currency: String,
    pub orders: i64,
    pub revenue_minor: i64,
}

//

impl Mailer {
    pub fn new(provider: EmailProvider) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self { provider, client })
    }

    /// Direct, unqueued send for login codes.
    pub async fn send_otp(&self, to: &str, code: &str) -> anyhow::Result<()> {
        if matches!(self.provider, EmailProvider::Log) {
            tracing::info!("OTP for {to}: {code}");
            return Ok(());
        }
        let text = format!("Your login code is {code}. It expires in 5 minutes.");
        let html = layout(
            "Your login code",
            &format!(
                r#"<p style="color:#555">Use this code to sign in. It expires in 5 minutes.</p>
                <div style="font-size:32px;letter-spacing:8px;font-weight:bold;text-align:center;padding:16px;background:#f6f6f6;border-radius:6px">{code}</div>
                <p style="color:#999;font-size:12px">If you didn't request this, ignore this email.</p>"#
            ),
        );
        self.send(to, "Your sales-app login code", &text, &html, None)
            .await
    }

    pub async fn send_email(&self, to: &str, email: &Email, key: &str) -> anyhow::Result<()> {
        let (subject, text, html) = email.render();
        self.send(to, &subject, &text, &html, Some(key)).await
    }

    async fn send(
        &self,
        to: &str,
        subject: &str,
        text: &str,
        html: &str,
        idempotency_key: Option<&str>,
    ) -> anyhow::Result<()> {
        match &self.provider {
            EmailProvider::Log => {
                tracing::info!("EMAIL to {to} | {subject}\n{text}");
                Ok(())
            }
            EmailProvider::Resend { api_key, from } => {
                let mut req = self
                    .client
                    .post("https://api.resend.com/emails")
                    .bearer_auth(api_key)
                    .json(&json!({
                        "from": from, "to": [to],
                        "subject": subject, "text": text, "html": html,
                    }));
                if let Some(k) = idempotency_key {
                    req = req.header("Idempotency-Key", k);
                }
                let res = req.send().await?;
                let status = res.status();
                if !status.is_success() {
                    let body = res.text().await.unwrap_or_default();
                    let permanent =
                        status.is_client_error() && !matches!(status.as_u16(), 408 | 429);
                    let tag = if permanent { "permanent: " } else { "" };
                    anyhow::bail!("{tag}resend returned {status}: {body}");
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Email {
    OrderCreated {
        order_id: Uuid,
        total_minor: i64,
        currency: String,
    },
    OrderDelivered {
        order_id: Uuid,
    },
    // in enum Email, replace DailyDigest:
    DailyDigest {
        date: String,
        totals: Vec<CurrencyTotal>,
    },
    // in enum Email, replace DailyDigest:
}

impl Email {
    pub fn kind(&self) -> &'static str {
        match self {
            Email::OrderCreated { .. } => "order_created",
            Email::OrderDelivered { .. } => "order_delivered",
            Email::DailyDigest { .. } => "daily_digest",
        }
    }

    /// Lower number is sent first, so analytics never delays order mail.
    pub fn priority(&self) -> i16 {
        match self {
            Email::OrderCreated { .. } | Email::OrderDelivered { .. } => 3,
            Email::DailyDigest { .. } => 8,
        }
    }

    pub fn render(&self) -> (String, String, String) {
        match self {
            Email::OrderCreated {
                order_id,
                total_minor,
                currency,
            } => {
                let total = money(*total_minor, currency);
                let short = &order_id.to_string()[..8];
                (
                    format!("Order {short} received"),
                    format!("We received your order {order_id}. Total: {total}."),
                    layout(
                        "Order received",
                        &format!(
                            r#"<p>We received your order <b>{short}</b>.</p><p>Total: <b>{total}</b></p>"#
                        ),
                    ),
                )
            }
            Email::OrderDelivered { order_id } => {
                let short = &order_id.to_string()[..8];
                (
                    format!("Order {short} delivered"),
                    format!("Your order {order_id} has been delivered."),
                    layout(
                        "Order delivered",
                        &format!(r#"<p>Your order <b>{short}</b> has been delivered.</p>"#),
                    ),
                )
            }

            Email::DailyDigest { date, totals } => {
                let text_rows = if totals.is_empty() {
                    "No orders.".to_string()
                } else {
                    totals
                        .iter()
                        .map(|t| {
                            format!(
                                "{}: {} orders, {}",
                                t.currency,
                                t.orders,
                                money(t.revenue_minor, &t.currency)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let html_rows = if totals.is_empty() {
                    "<p>No orders.</p>".to_string()
                } else {
                    totals
                        .iter()
                        .map(|t| {
                            format!(
                                "<p>{}: <b>{}</b> orders, <b>{}</b></p>",
                                t.currency,
                                t.orders,
                                money(t.revenue_minor, &t.currency)
                            )
                        })
                        .collect::<String>()
                };
                (
                    format!("Daily summary for {date}"),
                    format!("{date}\n{text_rows}"),
                    layout(&format!("Daily summary, {date}"), &html_rows),
                )
            }
        }
    }
}

fn money(minor: i64, currency: &str) -> String {
    format!("{}.{:02} {}", minor / 100, (minor % 100).abs(), currency)
        .trim()
        .to_string()
}

fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<div style="font-family:Arial,sans-serif;max-width:420px;margin:auto;padding:24px;border:1px solid #eee;border-radius:8px"><h2 style="margin:0 0 12px">{title}</h2>{body}</div>"#
    )
}
