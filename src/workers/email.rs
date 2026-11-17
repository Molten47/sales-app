use crate::domain::analytics;
use chrono::{Days, Timelike, Utc};
use sqlx::PgPool;
use std::time::Duration;

use crate::{
    domain::email_jobs,
    email::{Email, Mailer},
};

const DIGEST_HOUR_UTC: u32 = 7;

pub async fn run(db: PgPool, mailer: Mailer) {
    loop {
        match email_jobs::claim(&db).await {
            Ok(Some(job)) => {
                let result = match serde_json::from_value::<Email>(job.payload.clone()) {
                    Ok(email) => {
                        mailer
                            .send_email(&job.to_email, &email, &job.id.to_string())
                            .await
                    }
                    Err(e) => Err(anyhow::anyhow!("bad payload: {e}")),
                };
                let outcome = match result {
                    Ok(()) => {
                        tracing::info!(job = %job.id, to = %job.to_email, "email sent");
                        email_jobs::mark_sent(&db, job.id).await
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        tracing::error!(job = %job.id, "email failed: {msg}");
                        if msg.starts_with("permanent:") {
                            email_jobs::mark_dead(&db, job.id, &msg).await
                        } else {
                            email_jobs::mark_failed(&db, job.id, job.attempts, &msg).await
                        }
                    }
                };
                if let Err(e) = outcome {
                    tracing::error!("email job bookkeeping failed: {e}");
                }

                // stays under Resend's 2 requests/second limit
                tokio::time::sleep(Duration::from_millis(600)).await;
            }
            Ok(None) => tokio::time::sleep(Duration::from_secs(2)).await,
            Err(e) => {
                tracing::error!("email claim failed: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Hourly tick. The dedupe key makes it enqueue at most one digest per staff member per day.
pub async fn daily_digest(db: PgPool) {
    loop {
        if Utc::now().hour() >= DIGEST_HOUR_UTC
            && let Err(e) = enqueue_digest(&db).await
        {
            tracing::error!("digest failed: {e}");
        }

        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

async fn enqueue_digest(db: &PgPool) -> Result<(), sqlx::Error> {
    let today = Utc::now().date_naive();
    let day = today - Days::new(1);
    let start = day.and_hms_opt(0, 0, 0).unwrap().and_utc();
    let end = today.and_hms_opt(0, 0, 0).unwrap().and_utc();

    let totals = analytics::totals_between(db, start, end).await?;
    let date = day.to_string();
    let staff: Vec<String> = sqlx::query_scalar("SELECT email FROM users WHERE role = 'staff'")
        .fetch_all(db)
        .await?;

    for to in staff {
        let email = Email::DailyDigest {
            date: date.clone(),
            totals: totals.clone(),
        };
        email_jobs::enqueue(db, &to, &email, Some(&format!("digest:{date}:{to}"))).await?;
    }
    Ok(())
}
