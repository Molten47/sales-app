use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::email::CurrencyTotal;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DailyTotal {
    pub day: NaiveDate,
    pub currency: String,
    pub orders: i64,
    pub revenue_minor: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct StatusCount {
    pub status: String,
    pub orders: i64,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub generated_at: DateTime<Utc>,
    pub today: Vec<CurrencyTotal>,
    /// Only days and currencies that have orders; the frontend fills the gaps.
    pub last_7_days: Vec<DailyTotal>,
    pub by_status: Vec<StatusCount>,
}

fn day_start(d: NaiveDate) -> DateTime<Utc> {
    d.and_hms_opt(0, 0, 0).unwrap().and_utc()
}

/// Orders and revenue per currency in [start, end). Cancelled orders don't count.
/// Shared by the dashboard and the daily digest.
pub async fn totals_between(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<CurrencyTotal>, sqlx::Error> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT currency::text, count(*), COALESCE(sum(total_minor), 0)::bigint
         FROM orders
         WHERE created_at >= $1 AND created_at < $2 AND status <> 'cancelled'
         GROUP BY currency ORDER BY currency",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(currency, orders, revenue_minor)| CurrencyTotal {
            currency,
            orders,
            revenue_minor,
        })
        .collect())
}

pub async fn summary(pool: &PgPool) -> Result<Summary, sqlx::Error> {
    let today = Utc::now().date_naive();

    let today_totals =
        totals_between(pool, day_start(today), day_start(today + Days::new(1))).await?;

    let last_7_days = sqlx::query_as::<_, DailyTotal>(
        "SELECT (created_at AT TIME ZONE 'UTC')::date AS day,
                currency::text AS currency,
                count(*) AS orders,
                COALESCE(sum(total_minor), 0)::bigint AS revenue_minor
         FROM orders
         WHERE created_at >= $1 AND status <> 'cancelled'
         GROUP BY 1, 2 ORDER BY 1, 2",
    )
    .bind(day_start(today - Days::new(6)))
    .fetch_all(pool)
    .await?;

    let by_status = sqlx::query_as::<_, StatusCount>(
        "SELECT status, count(*) AS orders FROM orders GROUP BY status ORDER BY status",
    )
    .fetch_all(pool)
    .await?;

    Ok(Summary {
        generated_at: Utc::now(),
        today: today_totals,
        last_7_days,
        by_status,
    })
}
