use axum::{
    extract::{State, Path, FromRequestParts, Query},
    http::StatusCode,
    routing::{get, put, post, delete}, // Add delete route
    Json, Router,
};
use axum::http::request::Parts;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool, postgres::PgPoolOptions};
use tower_http::cors::CorsLayer;
use uuid::Uuid;
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{encode, Header, EncodingKey, decode, DecodingKey, Validation};

const FALLBACK_FREE_MAX_ROADMAPS: i64 = 3;
const FALLBACK_FREE_MAX_NODES_PER_ORG: i64 = 50;
const FALLBACK_FREE_MAX_MEMBERS_PER_ORG: i64 = 2;

const EVENT_ROADMAP_CREATED: &str = "roadmap_created";
const EVENT_NODE_CAP_HIT: &str = "node_cap_hit";
const EVENT_UPGRADE_MODAL_OPENED: &str = "upgrade_modal_opened";
const EVENT_CHECKOUT_STARTED: &str = "checkout_started";
const EVENT_CHECKOUT_SUCCEEDED: &str = "checkout_succeeded";
const EVENT_INVITE_SENT: &str = "invite_sent";
const EVENT_SHARED_LINK_COPIED: &str = "shared_link_copied";

const SUPPORTED_MARKET_CN: &str = "cn";
const SUPPORTED_MARKET_GLOBAL: &str = "global";

// ==========================================
// 1. 鏁版嵁妯″瀷瀹氫箟
// ==========================================

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub exp: usize,
}

#[derive(Deserialize)]
pub struct AuthReq {
    pub username: String,
    pub email: Option<String>,
    pub password: String,
    pub invite_code: Option<String>,
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Node {
    pub id: Uuid,
    pub roadmap_id: Option<Uuid>, 
    pub title: String,
    pub status: Option<String>,
    pub pos_x: f64,
    pub pos_y: f64,
}

#[derive(Deserialize)]
pub struct RoadmapQuery {
    pub roadmap_id: Uuid,
}

#[derive(Deserialize)]
pub struct CreateNodeReq {
    pub roadmap_id: Uuid,
    pub title: String,
    pub pos_x: f64,
    pub pos_y: f64,
}

#[derive(Deserialize)]
pub struct UpdateNodeReq {
    pub title: Option<String>,
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateNodePosReq {
    pub pos_x: f64,
    pub pos_y: f64,
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Edge {
    pub id: Uuid,
    pub roadmap_id: Option<Uuid>,
    pub source_node_id: Uuid,
    pub target_node_id: Uuid,
}

#[derive(Deserialize)]
pub struct CreateEdgeReq {
    pub roadmap_id: Uuid,
    pub source: Uuid,
    pub target: Uuid,
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Note {
    pub node_id: Uuid,
    pub content: serde_json::Value,
}

#[derive(Serialize)]
pub struct ShareNoteResponse {
    pub content: serde_json::Value,
    pub references: Vec<NodeReference>,
}

#[derive(Deserialize)]
pub struct UpdateNoteReq {
    pub content: serde_json::Value,
}

// Node reference model
#[derive(Serialize, Deserialize, FromRow)]
pub struct NodeReference {
    pub id: Uuid,
    pub node_id: Uuid,
    pub title: String,
    pub url: String,
}

#[derive(Deserialize)]
pub struct CreateReferenceReq {
    pub title: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Roadmap {
    pub id: Uuid,
    pub title: String,
    pub share_token: Option<String>,
}

#[derive(Serialize)]
pub struct ShareData {
    pub roadmap_title: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Serialize)]
pub struct OrgDetails {
    pub name: String,
    pub plan_type: String,
    pub billing_status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub billing_market: String,
    pub members: Vec<OrgMemberInfo>,
}

#[derive(Serialize, FromRow)]
pub struct OrgMemberInfo {
    pub id: Uuid,
    pub nickname: String,
    pub email: String,
    pub role: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize, FromRow, Clone)]
pub struct PlanEntitlement {
    pub plan_type: String,
    pub market: String,
    pub currency: String,
    pub price_cents: i32,
    pub billing_interval: String,
    pub max_roadmaps: Option<i64>,
    pub max_nodes_per_org: Option<i64>,
    pub max_members_per_org: Option<i64>,
    pub can_public_share: bool,
    pub priority_support: bool,
    pub sso_enabled: bool,
    pub audit_log_enabled: bool,
    pub private_deployment: bool,
}

#[derive(Serialize)]
pub struct BillingPlansResponse {
    pub generated_at: DateTime<Utc>,
    pub plans: Vec<PlanEntitlement>,
}

#[derive(Deserialize)]
pub struct CreateCheckoutSessionReq {
    pub plan_type: Option<String>,
    pub market: Option<String>,
    pub seats: Option<i32>,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
}

#[derive(Serialize, FromRow)]
pub struct CreateCheckoutSessionResp {
    pub external_session_id: String,
    pub checkout_url: String,
    pub provider: String,
    pub status: String,
}

#[derive(Deserialize)]
pub struct BillingWebhookReq {
    pub external_session_id: String,
    pub status: String,
    pub provider_event_id: Option<String>,
    pub current_period_end: Option<DateTime<Utc>>,
    pub raw_payload: Option<Value>,
}

#[derive(Serialize)]
pub struct BillingSubscriptionResp {
    pub org_id: Uuid,
    pub plan_type: String,
    pub billing_status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub market: String,
    pub entitlement: PlanEntitlement,
}

#[derive(Deserialize)]
pub struct TrackEventReq {
    pub name: String,
    pub properties: Option<Value>,
}

fn normalize_note_content_for_storage(content: Value) -> Value {
    match content {
        Value::String(markdown) => json!({ "markdown": markdown, "doc_json": null }),
        Value::Object(mut map) => {
            if !map.contains_key("markdown") {
                let markdown = map
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                map.insert("markdown".to_string(), Value::String(markdown));
            }

            if !map.contains_key("doc_json") {
                map.insert("doc_json".to_string(), Value::Null);
            }

            Value::Object(map)
        }
        Value::Null => json!({ "markdown": "", "doc_json": null }),
        other => json!({ "markdown": other.to_string(), "doc_json": null }),
    }
}

fn normalize_note_content_for_response(content: Value) -> Value {
    let normalized = normalize_note_content_for_storage(content);

    if let Value::Object(mut map) = normalized {
        if !matches!(map.get("markdown"), Some(Value::String(_))) {
            map.insert("markdown".to_string(), Value::String(String::new()));
        }

        if !map.contains_key("doc_json") {
            map.insert("doc_json".to_string(), Value::Null);
        }

        return Value::Object(map);
    }

    normalized
}

fn normalize_market(input: Option<&str>) -> String {
    match input.unwrap_or(SUPPORTED_MARKET_CN).to_lowercase().as_str() {
        SUPPORTED_MARKET_GLOBAL => SUPPORTED_MARKET_GLOBAL.to_string(),
        _ => SUPPORTED_MARKET_CN.to_string(),
    }
}

fn normalize_plan_type(input: Option<&str>) -> String {
    match input.unwrap_or("team").to_lowercase().as_str() {
        "enterprise" => "enterprise".to_string(),
        "free" => "free".to_string(),
        _ => "team".to_string(),
    }
}

fn default_entitlement(plan_type: &str, market: &str) -> PlanEntitlement {
    let currency = if market == SUPPORTED_MARKET_GLOBAL { "USD" } else { "CNY" };
    let team_price = if market == SUPPORTED_MARKET_GLOBAL { 900 } else { 3000 };

    match plan_type {
        "enterprise" => PlanEntitlement {
            plan_type: "enterprise".to_string(),
            market: market.to_string(),
            currency: currency.to_string(),
            price_cents: 0,
            billing_interval: "month".to_string(),
            max_roadmaps: None,
            max_nodes_per_org: None,
            max_members_per_org: None,
            can_public_share: true,
            priority_support: true,
            sso_enabled: true,
            audit_log_enabled: true,
            private_deployment: true,
        },
        "team" => PlanEntitlement {
            plan_type: "team".to_string(),
            market: market.to_string(),
            currency: currency.to_string(),
            price_cents: team_price,
            billing_interval: "month".to_string(),
            max_roadmaps: None,
            max_nodes_per_org: None,
            max_members_per_org: None,
            can_public_share: true,
            priority_support: true,
            sso_enabled: false,
            audit_log_enabled: false,
            private_deployment: false,
        },
        _ => PlanEntitlement {
            plan_type: "free".to_string(),
            market: market.to_string(),
            currency: currency.to_string(),
            price_cents: 0,
            billing_interval: "month".to_string(),
            max_roadmaps: Some(FALLBACK_FREE_MAX_ROADMAPS),
            max_nodes_per_org: Some(FALLBACK_FREE_MAX_NODES_PER_ORG),
            max_members_per_org: Some(FALLBACK_FREE_MAX_MEMBERS_PER_ORG),
            can_public_share: true,
            priority_support: false,
            sso_enabled: false,
            audit_log_enabled: false,
            private_deployment: false,
        },
    }
}

async fn fetch_entitlement(pool: &PgPool, plan_type: &str, market: &str) -> Result<PlanEntitlement, (StatusCode, String)> {
    let selected = match sqlx::query_as::<_, PlanEntitlement>(
        r#"SELECT plan_type, market, currency, price_cents, billing_interval,
                  max_roadmaps, max_nodes_per_org, max_members_per_org,
                  can_public_share, priority_support, sso_enabled, audit_log_enabled, private_deployment
           FROM plan_entitlements
           WHERE plan_type = $1 AND market = $2
           LIMIT 1"#,
    )
    .bind(plan_type)
    .bind(market)
    .fetch_optional(pool)
    .await {
        Ok(v) => v,
        Err(err) => {
            // Missing table/legacy schema fallback: keep critical flows alive with baked-in defaults.
            let msg = err.to_string();
            if msg.contains("plan_entitlements") {
                eprintln!("billing fallback activated: {msg}");
                return Ok(default_entitlement(plan_type, market));
            }
            return Err((StatusCode::INTERNAL_SERVER_ERROR, msg));
        }
    };

    if let Some(entitlement) = selected {
        return Ok(entitlement);
    }

    let fallback_global = match sqlx::query_as::<_, PlanEntitlement>(
        r#"SELECT plan_type, market, currency, price_cents, billing_interval,
                  max_roadmaps, max_nodes_per_org, max_members_per_org,
                  can_public_share, priority_support, sso_enabled, audit_log_enabled, private_deployment
           FROM plan_entitlements
           WHERE plan_type = $1 AND market = $2
           LIMIT 1"#,
    )
    .bind(plan_type)
    .bind(SUPPORTED_MARKET_GLOBAL)
    .fetch_optional(pool)
    .await {
        Ok(v) => v,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("plan_entitlements") {
                eprintln!("billing global fallback activated: {msg}");
                None
            } else {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, msg));
            }
        }
    };

    Ok(fallback_global.unwrap_or_else(|| default_entitlement(plan_type, market)))
}

async fn resolve_org_context(pool: &PgPool, user_id: Uuid) -> Result<(Uuid, String, String, String, Option<DateTime<Utc>>), (StatusCode, String)> {
    match sqlx::query_as(
        r#"SELECT o.id,
                  o.plan_type,
                  COALESCE(o.billing_market, $2) AS billing_market,
                  COALESCE(o.billing_status, 'inactive') AS billing_status,
                  o.current_period_end
           FROM organizations o
           JOIN org_members om ON o.id = om.org_id
           WHERE om.user_id = $1
           LIMIT 1"#,
    )
    .bind(user_id)
    .bind(SUPPORTED_MARKET_CN)
    .fetch_optional(pool)
    .await {
        Ok(row) => row.ok_or((StatusCode::NOT_FOUND, "Organization not found".to_string())),
        Err(err) => {
            let msg = err.to_string();
            // Legacy schema fallback: organizations table may not contain billing columns yet.
            if msg.contains("billing_market") || msg.contains("billing_status") || msg.contains("current_period_end") {
                eprintln!("org context legacy fallback activated: {msg}");
                let row: Option<(Uuid, String)> = sqlx::query_as(
                    r#"SELECT o.id, o.plan_type
                       FROM organizations o
                       JOIN org_members om ON o.id = om.org_id
                       WHERE om.user_id = $1
                       LIMIT 1"#,
                )
                .bind(user_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                if let Some((id, plan)) = row {
                    return Ok((id, plan, SUPPORTED_MARKET_CN.to_string(), "inactive".to_string(), None));
                }
                return Err((StatusCode::NOT_FOUND, "Organization not found".to_string()));
            }
            Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
        }
    }
}

fn is_track_event_allowed(name: &str) -> bool {
    matches!(
        name,
        EVENT_ROADMAP_CREATED
            | EVENT_NODE_CAP_HIT
            | EVENT_UPGRADE_MODAL_OPENED
            | EVENT_CHECKOUT_STARTED
            | EVENT_CHECKOUT_SUCCEEDED
            | EVENT_INVITE_SENT
            | EVENT_SHARED_LINK_COPIED
    )
}

async fn record_event(pool: &PgPool, user_id: Option<Uuid>, org_id: Option<Uuid>, name: &str, properties: Value) {
    if !is_track_event_allowed(name) {
        return;
    }

    let _ = sqlx::query(
        "INSERT INTO product_events (name, org_id, user_id, properties) VALUES ($1, $2, $3, $4)",
    )
    .bind(name)
    .bind(org_id)
    .bind(user_id)
    .bind(properties)
    .execute(pool)
    .await;
}

// JWT extractor
#[axum::async_trait]
impl<S> FromRequestParts<S> for Claims
where S: Send + Sync,
{
    type Rejection = (StatusCode, String);
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts.headers.get("Authorization").and_then(|h| h.to_str().ok()).ok_or((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;
        if !auth_header.starts_with("Bearer ") { return Err((StatusCode::UNAUTHORIZED, "Invalid token format".to_string())); }
        let token = &auth_header[7..];
        let token_data = decode::<Claims>(token, &DecodingKey::from_secret("secret".as_ref()), &Validation::default())
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Session expired".to_string()))?;
        Ok(token_data.claims)
    }
}

// ==========================================
// 2. 涓氬姟閫昏緫 (Handlers)
// ==========================================

// --- Roadmap handlers ---

async fn update_roadmap(
    claims: Claims,
    Path(id): Path<Uuid>,
    State(pool): State<PgPool>,
    Json(payload): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    let title = payload["title"].as_str().ok_or((StatusCode::BAD_REQUEST, "Title is required".to_string()))?;
    // 鍙湁缁勭粐绠＄悊鍛樻垨缂栬緫鑰呭彲浠ヤ慨鏀硅矾绾垮浘鍚嶇О
    let query = "
        UPDATE roadmaps SET title = $1 
        WHERE id = $2 AND org_id IN (
            SELECT org_id FROM org_members WHERE user_id = $3 AND role IN ('admin', 'editor')
        )
    ";
    let res = sqlx::query(query).bind(title).bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    if res.rows_affected() > 0 { Ok(StatusCode::OK) } else { Err((StatusCode::FORBIDDEN, "Forbidden or not found".to_string())) }
}

// --- Node reference handlers ---

async fn get_node_references(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>) -> Result<Json<Vec<NodeReference>>, (StatusCode, String)> {
    let res = sqlx::query_as::<_, NodeReference>(
        r#"SELECT nr.id, nr.node_id, nr.title, nr.url
           FROM node_references nr
           JOIN nodes n ON nr.node_id = n.id
           JOIN roadmaps r ON n.roadmap_id = r.id
           JOIN org_members om ON r.org_id = om.org_id
           WHERE nr.node_id = $1 AND om.user_id = $2
           ORDER BY nr.created_at DESC"#,
    )
        .bind(id)
        .bind(claims.sub)
        .fetch_all(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(res))
}

async fn get_shared_node_references(Path((token, node_id)): Path<(String, Uuid)>, State(pool): State<PgPool>) -> Result<Json<Vec<NodeReference>>, (StatusCode, String)> {
    let references = sqlx::query_as::<_, NodeReference>(
        r#"SELECT nr.id, nr.node_id, nr.title, nr.url
           FROM node_references nr
           JOIN nodes nd ON nr.node_id = nd.id
           JOIN roadmaps r ON nd.roadmap_id = r.id
           WHERE r.share_token = $1 AND nr.node_id = $2
           ORDER BY nr.created_at DESC"#,
    )
        .bind(token)
        .bind(node_id)
        .fetch_all(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(references))
}

async fn add_node_reference(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>, Json(payload): Json<CreateReferenceReq>) -> Result<Json<NodeReference>, (StatusCode, String)> {
    let res = sqlx::query_as::<_, NodeReference>(
        r#"INSERT INTO node_references (node_id, title, url)
           SELECT $1, $2, $3
           WHERE EXISTS (
              SELECT 1
              FROM nodes n
              JOIN roadmaps r ON n.roadmap_id = r.id
              JOIN org_members om ON r.org_id = om.org_id
              WHERE n.id = $1 AND om.user_id = $4
           )
           RETURNING id, node_id, title, url"#,
    )
        .bind(id)
        .bind(payload.title)
        .bind(payload.url)
        .bind(claims.sub)
        .fetch_optional(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::FORBIDDEN, "无权访问".to_string()))?;
    Ok(Json(res))
}

async fn delete_node_reference(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>) -> Result<StatusCode, (StatusCode, String)> {
    let res = sqlx::query(
        r#"DELETE FROM node_references
           WHERE id = $1 AND node_id IN (
              SELECT n.id
              FROM nodes n
              JOIN roadmaps r ON n.roadmap_id = r.id
              JOIN org_members om ON r.org_id = om.org_id
              WHERE om.user_id = $2
           )"#,
    )
        .bind(id)
        .bind(claims.sub)
        .execute(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if res.rows_affected() > 0 {
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::FORBIDDEN, "无权访问".to_string()))
    }
}

async fn get_billing_plans(State(pool): State<PgPool>) -> Result<Json<BillingPlansResponse>, (StatusCode, String)> {
    let plans = match sqlx::query_as::<_, PlanEntitlement>(
        r#"SELECT plan_type, market, currency, price_cents, billing_interval,
                  max_roadmaps, max_nodes_per_org, max_members_per_org,
                  can_public_share, priority_support, sso_enabled, audit_log_enabled, private_deployment
           FROM plan_entitlements
           ORDER BY market ASC,
                    CASE plan_type
                        WHEN 'free' THEN 1
                        WHEN 'team' THEN 2
                        WHEN 'enterprise' THEN 3
                        ELSE 99
                    END ASC"#,
    )
    .fetch_all(&pool)
    .await {
        Ok(v) => v,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("plan_entitlements") {
                eprintln!("billing plans fallback activated: {msg}");
                Vec::new()
            } else {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, msg));
            }
        }
    };

    let resolved_plans = if plans.is_empty() {
        vec![
            default_entitlement("free", SUPPORTED_MARKET_CN),
            default_entitlement("team", SUPPORTED_MARKET_CN),
            default_entitlement("enterprise", SUPPORTED_MARKET_CN),
            default_entitlement("free", SUPPORTED_MARKET_GLOBAL),
            default_entitlement("team", SUPPORTED_MARKET_GLOBAL),
            default_entitlement("enterprise", SUPPORTED_MARKET_GLOBAL),
        ]
    } else {
        plans
    };

    Ok(Json(BillingPlansResponse {
        generated_at: Utc::now(),
        plans: resolved_plans,
    }))
}

async fn get_billing_subscription(claims: Claims, State(pool): State<PgPool>) -> Result<Json<BillingSubscriptionResp>, (StatusCode, String)> {
    let (org_id, plan_type, market, billing_status, current_period_end) = resolve_org_context(&pool, claims.sub).await?;
    let entitlement = fetch_entitlement(&pool, &plan_type, &market).await?;
    Ok(Json(BillingSubscriptionResp {
        org_id,
        plan_type,
        billing_status,
        current_period_end,
        market,
        entitlement,
    }))
}

async fn create_checkout_session(
    claims: Claims,
    State(pool): State<PgPool>,
    Json(payload): Json<CreateCheckoutSessionReq>,
) -> Result<Json<CreateCheckoutSessionResp>, (StatusCode, String)> {
    let (org_id, _, current_market, _, _) = resolve_org_context(&pool, claims.sub).await?;
    let target_plan = normalize_plan_type(payload.plan_type.as_deref());
    if target_plan == "free" {
        return Err((StatusCode::BAD_REQUEST, "Checkout is only available for paid plans".to_string()));
    }

    let market = normalize_market(payload.market.as_deref().or(Some(current_market.as_str())));
    let seats = payload.seats.unwrap_or(1).max(1);
    let entitlement = fetch_entitlement(&pool, &target_plan, &market).await?;
    let amount_cents = entitlement.price_cents.saturating_mul(seats);

    let external_session_id = format!("chk_{}", Uuid::new_v4().simple());
    let checkout_url = format!(
        "https://billing.pathio.local/checkout/{}?plan={}&market={}&seats={}",
        external_session_id, target_plan, market, seats
    );
    let provider = "mock_gateway".to_string();
    let status = "pending".to_string();

    let created = sqlx::query_as::<_, CreateCheckoutSessionResp>(
        r#"INSERT INTO billing_checkout_sessions
           (org_id, plan_type, market, currency, seats, amount_cents, provider, external_session_id, checkout_url, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING external_session_id, checkout_url, provider, status"#,
    )
    .bind(org_id)
    .bind(&target_plan)
    .bind(&market)
    .bind(&entitlement.currency)
    .bind(seats)
    .bind(amount_cents)
    .bind(&provider)
    .bind(&external_session_id)
    .bind(&checkout_url)
    .bind(&status)
    .fetch_one(&pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    record_event(
        &pool,
        Some(claims.sub),
        Some(org_id),
        EVENT_CHECKOUT_STARTED,
        json!({
            "external_session_id": external_session_id,
            "plan_type": target_plan,
            "market": market,
            "seats": seats,
            "amount_cents": amount_cents,
            "success_url": payload.success_url,
            "cancel_url": payload.cancel_url
        }),
    )
    .await;

    Ok(Json(created))
}

async fn billing_webhook(State(pool): State<PgPool>, Json(payload): Json<BillingWebhookReq>) -> Result<StatusCode, (StatusCode, String)> {
    let status = payload.status.to_lowercase();
    if !matches!(status.as_str(), "paid" | "failed" | "canceled" | "refunded") {
        return Err((StatusCode::BAD_REQUEST, "Unsupported billing status".to_string()));
    }

    let session: (Uuid, String, String) = sqlx::query_as(
        "SELECT org_id, plan_type, market FROM billing_checkout_sessions WHERE external_session_id = $1",
    )
    .bind(&payload.external_session_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or((StatusCode::NOT_FOUND, "Checkout session not found".to_string()))?;

    sqlx::query(
        r#"UPDATE billing_checkout_sessions
           SET status = $1,
               provider_event_id = COALESCE($2, provider_event_id),
               raw_payload = COALESCE($3, raw_payload),
               updated_at = CURRENT_TIMESTAMP
           WHERE external_session_id = $4"#,
    )
    .bind(&status)
    .bind(payload.provider_event_id)
    .bind(payload.raw_payload)
    .bind(&payload.external_session_id)
    .execute(&pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match status.as_str() {
        "paid" => {
            let period_end = payload.current_period_end.unwrap_or_else(|| Utc::now() + Duration::days(30));
            sqlx::query(
                r#"UPDATE organizations
                   SET plan_type = $1,
                       billing_status = 'active',
                       billing_market = $2,
                       current_period_end = $3
                   WHERE id = $4"#,
            )
            .bind(&session.1)
            .bind(&session.2)
            .bind(period_end)
            .bind(session.0)
            .execute(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            record_event(
                &pool,
                None,
                Some(session.0),
                EVENT_CHECKOUT_SUCCEEDED,
                json!({
                    "external_session_id": payload.external_session_id,
                    "plan_type": session.1,
                    "market": session.2,
                    "current_period_end": period_end
                }),
            )
            .await;
        }
        "failed" => {
            sqlx::query("UPDATE organizations SET billing_status = 'past_due' WHERE id = $1")
                .bind(session.0)
                .execute(&pool)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        _ => {
            sqlx::query(
                "UPDATE organizations SET plan_type = 'free', billing_status = 'inactive', current_period_end = NULL WHERE id = $1",
            )
            .bind(session.0)
            .execute(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
    }

    Ok(StatusCode::OK)
}

async fn track_event(claims: Claims, State(pool): State<PgPool>, Json(payload): Json<TrackEventReq>) -> Result<StatusCode, (StatusCode, String)> {
    if !is_track_event_allowed(payload.name.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "Event is not in the allowlist".to_string()));
    }

    let org_id = sqlx::query_scalar::<_, Uuid>("SELECT org_id FROM org_members WHERE user_id = $1 LIMIT 1")
        .bind(claims.sub)
        .fetch_optional(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    record_event(
        &pool,
        Some(claims.sub),
        org_id,
        payload.name.as_str(),
        payload.properties.unwrap_or_else(|| json!({})),
    )
    .await;

    Ok(StatusCode::ACCEPTED)
}

async fn register(State(pool): State<PgPool>, Json(payload): Json<AuthReq>) -> Result<StatusCode, (StatusCode, String)> {
    let email = payload.email.ok_or((StatusCode::BAD_REQUEST, "Email is required".to_string()))?;
    let hashed = hash(payload.password, DEFAULT_COST).unwrap();
    let mut tx = pool.begin().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let user_id: Uuid = sqlx::query_scalar("INSERT INTO users (nickname, email, password_hash) VALUES ($1, $2, $3) RETURNING id")
        .bind(payload.username).bind(email).bind(hashed).fetch_one(&mut *tx).await
        .map_err(|_| (StatusCode::BAD_REQUEST, "Username or email already exists".to_string()))?;
    if let Some(code) = payload.invite_code {
        let org_info: Option<(Uuid, String, String)> = sqlx::query_as(
            "SELECT o.id, o.plan_type, COALESCE(o.billing_market, $2) FROM organizations o JOIN invitations i ON o.id = i.org_id WHERE i.code = $1 AND i.is_used = FALSE",
        )
        .bind(&code)
        .bind(SUPPORTED_MARKET_CN)
        .fetch_optional(&mut *tx)
        .await
        .unwrap();
        if let Some((oid, plan, market)) = org_info {
            let member_limit = fetch_entitlement(&pool, &plan, &market).await?.max_members_per_org;
            if let Some(limit) = member_limit {
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM org_members WHERE org_id = $1").bind(oid).fetch_one(&mut *tx).await.unwrap();
                if count >= limit { return Err((StatusCode::PAYMENT_REQUIRED, "Workspace member limit reached".to_string())); }
            }
            sqlx::query("INSERT INTO org_members (org_id, user_id, role) VALUES ($1, $2, 'member')").bind(oid).bind(user_id).execute(&mut *tx).await.unwrap();
            sqlx::query("UPDATE invitations SET is_used = TRUE WHERE code = $1").bind(&code).execute(&mut *tx).await.unwrap();
        } else { return Err((StatusCode::BAD_REQUEST, "Invalid invite code".to_string())); }
    } else {
    let org_id: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (name, owner_id, plan_type, billing_status, billing_market) VALUES ($1, $2, 'free', 'inactive', $3) RETURNING id",
    ).bind("My Workspace").bind(user_id).bind(SUPPORTED_MARKET_CN).fetch_one(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO org_members (org_id, user_id, role) VALUES ($1, $2, 'admin')").bind(org_id).bind(user_id).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO roadmaps (org_id, title, share_token) VALUES ($1, $2, $3)").bind(org_id).bind("My First Roadmap").bind(Uuid::new_v4().to_string()[..8].to_string()).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    Ok(StatusCode::CREATED)
}

async fn login(State(pool): State<PgPool>, Json(payload): Json<AuthReq>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (id, hash_val): (Uuid, String) = sqlx::query_as("SELECT id, password_hash FROM users WHERE nickname = $1").bind(payload.username).fetch_optional(&pool).await.unwrap().ok_or((StatusCode::UNAUTHORIZED, "User not found".to_string()))?;
    if verify(payload.password, &hash_val).unwrap() {
        let claims = Claims { sub: id, exp: 10000000000 }; 
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret("secret".as_ref())).unwrap();
        Ok(Json(serde_json::json!({ "token": token })))
    } else { Err((StatusCode::UNAUTHORIZED, "Incorrect password".to_string())) }
}

async fn get_org_details(claims: Claims, State(pool): State<PgPool>) -> Result<Json<OrgDetails>, (StatusCode, String)> {
    let org_res = sqlx::query_as::<_, (String, String, Uuid, String, Option<DateTime<Utc>>, String)>(
        "SELECT o.name, o.plan_type, o.id, COALESCE(o.billing_status, 'inactive') AS billing_status, o.current_period_end, COALESCE(o.billing_market, $2) AS billing_market FROM organizations o JOIN org_members om ON o.id = om.org_id WHERE om.user_id = $1 LIMIT 1",
    )
    .bind(claims.sub)
    .bind(SUPPORTED_MARKET_CN)
    .fetch_one(&pool)
    .await;

    let org: (String, String, Uuid, String, Option<DateTime<Utc>>, String) = match org_res {
        Ok(v) => v,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("billing_market") || msg.contains("billing_status") || msg.contains("current_period_end") {
                eprintln!("org details legacy fallback activated: {msg}");
                let legacy: (String, String, Uuid) = sqlx::query_as(
                    "SELECT o.name, o.plan_type, o.id FROM organizations o JOIN org_members om ON o.id = om.org_id WHERE om.user_id = $1 LIMIT 1",
                )
                .bind(claims.sub)
                .fetch_one(&pool)
                .await
                .map_err(|_| (StatusCode::NOT_FOUND, "Organization not found".to_string()))?;
                (legacy.0, legacy.1, legacy.2, "inactive".to_string(), None, SUPPORTED_MARKET_CN.to_string())
            } else {
                return Err((StatusCode::NOT_FOUND, "Organization not found".to_string()));
            }
        }
    };
    let members = sqlx::query_as::<_, OrgMemberInfo>("SELECT u.id, u.nickname, u.email, om.role, u.created_at FROM users u JOIN org_members om ON u.id = om.user_id WHERE om.org_id = $1").bind(org.2).fetch_all(&pool).await.unwrap();
    Ok(Json(OrgDetails {
        name: org.0,
        plan_type: org.1,
        billing_status: org.3,
        current_period_end: org.4,
        billing_market: org.5,
        members,
    }))
}

// Update workspace name
async fn update_org_details(
    claims: Claims,
    State(pool): State<PgPool>,
    Json(payload): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    let new_name = payload["name"].as_str()
        .ok_or((StatusCode::BAD_REQUEST, "Name is required".to_string()))?;

    // Only workspace admin can rename workspace
    let query = "
        UPDATE organizations SET name = $1 
        WHERE id = (
            SELECT org_id FROM org_members 
            WHERE user_id = $2 AND role = 'admin' 
            LIMIT 1
        )
    ";
    
    let res = sqlx::query(query)
        .bind(new_name)
        .bind(claims.sub)
        .execute(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if res.rows_affected() > 0 {
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::FORBIDDEN, "You do not have permission to rename this workspace".to_string()))
    }
}

async fn create_org_invite(claims: Claims, State(pool): State<PgPool>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let org: (Uuid, String, String) = sqlx::query_as(
        "SELECT org_id, o.plan_type, COALESCE(o.billing_market, $2) FROM org_members om JOIN organizations o ON om.org_id = o.id WHERE om.user_id = $1 AND om.role = 'admin'",
    )
    .bind(claims.sub)
    .bind(SUPPORTED_MARKET_CN)
    .fetch_one(&pool)
    .await
    .map_err(|_| (StatusCode::FORBIDDEN, "Permission denied".to_string()))?;

    let entitlement = fetch_entitlement(&pool, &org.1, &org.2).await?;
    if let Some(limit) = entitlement.max_members_per_org {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM org_members WHERE org_id = $1").bind(org.0).fetch_one(&pool).await.unwrap();
        if count >= limit { return Err((StatusCode::PAYMENT_REQUIRED, "Workspace member limit reached".to_string())); }
    }

    let code = Uuid::new_v4().to_string()[..6].to_uppercase();
    sqlx::query("INSERT INTO invitations (org_id, inviter_id, code) VALUES ($1, $2, $3)").bind(org.0).bind(claims.sub).bind(&code).execute(&pool).await.unwrap();

    record_event(
        &pool,
        Some(claims.sub),
        Some(org.0),
        EVENT_INVITE_SENT,
        json!({ "code": code.clone() }),
    )
    .await;

    Ok(Json(serde_json::json!({ "code": code })))
}

async fn create_roadmap(claims: Claims, State(pool): State<PgPool>, Json(payload): Json<serde_json::Value>) -> Result<Json<Roadmap>, (StatusCode, String)> {
    let title = payload["title"].as_str().unwrap_or("Untitled roadmap");
    let mut tx = pool.begin().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let org: (Uuid, String, String) = sqlx::query_as("SELECT o.id, o.plan_type, COALESCE(o.billing_market, $2) FROM organizations o JOIN org_members om ON o.id = om.org_id WHERE om.user_id = $1 LIMIT 1")
        .bind(claims.sub)
        .bind(SUPPORTED_MARKET_CN)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::FORBIDDEN, "Permission denied".to_string()))?;
    let entitlement = fetch_entitlement(&pool, &org.1, &org.2).await?;

    // Serialize quota checks inside a txn to avoid concurrent limit bypass.
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM organizations WHERE id = $1 FOR UPDATE")
        .bind(org.0)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(limit) = entitlement.max_roadmaps {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roadmaps WHERE org_id = $1")
            .bind(org.0)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if count >= limit {
            return Err((StatusCode::PAYMENT_REQUIRED, format!("Current plan is limited to {} roadmaps", limit)));
        }
    }

    let res = sqlx::query_as::<_, Roadmap>("INSERT INTO roadmaps (org_id, title, share_token) VALUES ($1, $2, $3) RETURNING id, title, share_token")
        .bind(org.0)
        .bind(title)
        .bind(Uuid::new_v4().to_string()[..8].to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tx.commit().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    record_event(
        &pool,
        Some(claims.sub),
        Some(org.0),
        EVENT_ROADMAP_CREATED,
        json!({ "roadmap_id": res.id, "title": res.title.clone() }),
    )
    .await;

    Ok(Json(res))
}

async fn get_roadmaps(claims: Claims, State(pool): State<PgPool>) -> Result<Json<Vec<Roadmap>>, (StatusCode, String)> {
    let res = sqlx::query_as::<_, Roadmap>("SELECT r.id, r.title, r.share_token FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $1 ORDER BY r.created_at DESC").bind(claims.sub).fetch_all(&pool).await.unwrap();
    Ok(Json(res))
}

async fn get_all_nodes(claims: Claims, Query(q): Query<RoadmapQuery>, State(pool): State<PgPool>) -> Result<Json<Vec<Node>>, (StatusCode, String)> {
    let res = sqlx::query_as::<_, Node>("SELECT n.* FROM nodes n JOIN roadmaps r ON n.roadmap_id = r.id JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $1 AND r.id = $2").bind(claims.sub).bind(q.roadmap_id).fetch_all(&pool).await.unwrap();
    Ok(Json(res))
}

async fn create_node(claims: Claims, State(pool): State<PgPool>, Json(payload): Json<CreateNodeReq>) -> Result<(StatusCode, Json<Node>), (StatusCode, String)> {
    let mut tx = pool.begin().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let org_and_plan: (Uuid, String, String) = sqlx::query_as(
        "SELECT r.org_id, o.plan_type, COALESCE(o.billing_market, $3) FROM roadmaps r JOIN organizations o ON r.org_id = o.id JOIN org_members om ON r.org_id = om.org_id WHERE r.id = $1 AND om.user_id = $2 LIMIT 1",
    )
        .bind(payload.roadmap_id)
        .bind(claims.sub)
        .bind(SUPPORTED_MARKET_CN)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::FORBIDDEN, "Forbidden".to_string()))?;
    let entitlement = fetch_entitlement(&pool, &org_and_plan.1, &org_and_plan.2).await?;

    // Serialize quota checks inside a txn to avoid concurrent limit bypass.
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM organizations WHERE id = $1 FOR UPDATE")
        .bind(org_and_plan.0)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(limit) = entitlement.max_nodes_per_org {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes n JOIN roadmaps r ON n.roadmap_id = r.id WHERE r.org_id = $1")
            .bind(org_and_plan.0)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if count >= limit {
            record_event(
                &pool,
                Some(claims.sub),
                Some(org_and_plan.0),
                EVENT_NODE_CAP_HIT,
                json!({ "roadmap_id": payload.roadmap_id, "current_count": count, "limit": limit }),
            )
            .await;
            return Err((StatusCode::PAYMENT_REQUIRED, format!("Current plan is limited to {} total nodes per workspace", limit)));
        }
    }

    let res = sqlx::query_as::<_, Node>("INSERT INTO nodes (roadmap_id, title, pos_x, pos_y) VALUES ($1, $2, $3, $4) RETURNING *")
        .bind(payload.roadmap_id)
        .bind(payload.title)
        .bind(payload.pos_x)
        .bind(payload.pos_y)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tx.commit().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(res)))
}

async fn update_node(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>, Json(payload): Json<UpdateNodeReq>) -> Result<StatusCode, (StatusCode, String)> {
    sqlx::query("UPDATE nodes SET title = COALESCE($1, title), status = COALESCE($2, status) WHERE id = $3 AND roadmap_id IN (SELECT r.id FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $4)").bind(payload.title).bind(payload.status).bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    Ok(StatusCode::OK)
}

async fn update_node_position(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>, Json(payload): Json<UpdateNodePosReq>) -> Result<StatusCode, (StatusCode, String)> {
    let res = sqlx::query("UPDATE nodes SET pos_x = $1, pos_y = $2 WHERE id = $3 AND roadmap_id IN (SELECT r.id FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $4)").bind(payload.pos_x).bind(payload.pos_y).bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    if res.rows_affected() > 0 { Ok(StatusCode::OK) } else { Err((StatusCode::FORBIDDEN, "Operation failed".to_string())) }
}

async fn delete_node(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>) -> Result<StatusCode, (StatusCode, String)> {
    sqlx::query("DELETE FROM nodes WHERE id = $1 AND roadmap_id IN (SELECT r.id FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $2)").bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    Ok(StatusCode::OK)
}

async fn get_all_edges(claims: Claims, Query(q): Query<RoadmapQuery>, State(pool): State<PgPool>) -> Result<Json<Vec<Edge>>, (StatusCode, String)> {
    let res = sqlx::query_as::<_, Edge>("SELECT e.* FROM edges e JOIN roadmaps r ON e.roadmap_id = r.id JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $1 AND r.id = $2").bind(claims.sub).bind(q.roadmap_id).fetch_all(&pool).await.unwrap();
    Ok(Json(res))
}

async fn create_edge(claims: Claims, State(pool): State<PgPool>, Json(payload): Json<CreateEdgeReq>) -> Result<StatusCode, (StatusCode, String)> {
    let query = "INSERT INTO edges (roadmap_id, source_node_id, target_node_id) SELECT $1, $2, $3 WHERE EXISTS (SELECT 1 FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE r.id = $1 AND om.user_id = $4) ON CONFLICT DO NOTHING";
    sqlx::query(query).bind(payload.roadmap_id).bind(payload.source).bind(payload.target).bind(claims.sub).execute(&pool).await.unwrap();
    Ok(StatusCode::CREATED)
}

async fn get_node_note(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>) -> Result<Json<Note>, (StatusCode, String)> {
    let mut res = sqlx::query_as::<_, Note>("INSERT INTO notes (node_id, content) SELECT $1, '{\"markdown\":\"\",\"doc_json\":null}' WHERE EXISTS (SELECT 1 FROM nodes n JOIN roadmaps r ON n.roadmap_id = r.id JOIN org_members om ON r.org_id = om.org_id WHERE n.id = $1 AND om.user_id = $2) ON CONFLICT (node_id) DO UPDATE SET node_id = EXCLUDED.node_id RETURNING node_id, content").bind(id).bind(claims.sub).fetch_one(&pool).await.map_err(|_| (StatusCode::FORBIDDEN, "Forbidden".to_string()))?;
    res.content = normalize_note_content_for_response(res.content);
    Ok(Json(res))
}

async fn update_node_note(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>, Json(payload): Json<UpdateNoteReq>) -> Result<StatusCode, (StatusCode, String)> {
    let query = "UPDATE notes SET content = $1, updated_at = CURRENT_TIMESTAMP WHERE node_id = $2 AND node_id IN (SELECT n.id FROM nodes n JOIN roadmaps r ON n.roadmap_id = r.id JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $3)";
    let normalized_content = normalize_note_content_for_storage(payload.content);
    sqlx::query(query).bind(normalized_content).bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    Ok(StatusCode::OK)
}

async fn get_shared_note(Path((token, node_id)): Path<(String, Uuid)>, State(pool): State<PgPool>) -> Result<Json<ShareNoteResponse>, (StatusCode, String)> {
    let node_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
               FROM nodes nd
               JOIN roadmaps r ON nd.roadmap_id = r.id
               WHERE r.share_token = $1 AND nd.id = $2
           )"#,
    )
        .bind(&token)
        .bind(node_id)
        .fetch_one(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !node_exists {
        return Err((StatusCode::FORBIDDEN, "无权访问".to_string()));
    }

    let content = sqlx::query_scalar::<_, serde_json::Value>("SELECT content FROM notes WHERE node_id = $1")
        .bind(node_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .unwrap_or_else(|| json!({ "markdown": "", "doc_json": null }));

    let normalized_content = normalize_note_content_for_response(content);

    let references = sqlx::query_as::<_, NodeReference>(
        r#"SELECT nr.id, nr.node_id, nr.title, nr.url
           FROM node_references nr
           JOIN nodes nd ON nr.node_id = nd.id
           JOIN roadmaps r ON nd.roadmap_id = r.id
           WHERE r.share_token = $1 AND nr.node_id = $2
           ORDER BY nr.created_at DESC"#,
    )
        .bind(&token)
        .bind(node_id)
        .fetch_all(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ShareNoteResponse { content: normalized_content, references }))
}

async fn get_shared_roadmap(Path(token): Path<String>, State(pool): State<PgPool>) -> Result<Json<ShareData>, (StatusCode, String)> {
    let roadmap = sqlx::query_as::<_, Roadmap>("SELECT id, title, share_token FROM roadmaps WHERE share_token = $1").bind(&token).fetch_optional(&pool).await.unwrap().ok_or((StatusCode::NOT_FOUND, "Invalid share token".to_string()))?;
    let nodes = sqlx::query_as::<_, Node>("SELECT * FROM nodes WHERE roadmap_id = $1").bind(roadmap.id).fetch_all(&pool).await.unwrap();
    let edges = sqlx::query_as::<_, Edge>("SELECT * FROM edges WHERE roadmap_id = $1").bind(roadmap.id).fetch_all(&pool).await.unwrap();
    Ok(Json(ShareData { roadmap_title: roadmap.title, nodes, edges }))
}

async fn health_check() -> &'static str { "Pathio API Running" }

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let pool = PgPoolOptions::new().max_connections(5).connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap();
    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/nodes", get(get_all_nodes).post(create_node))
        .route("/api/nodes/:id/position", put(update_node_position))
        .route("/api/edges", get(get_all_edges).post(create_edge))
        .route("/api/nodes/:id/note", get(get_node_note).put(update_node_note))
        .route("/api/share/:token", get(get_shared_roadmap))
        .route("/api/roadmaps", get(get_roadmaps).post(create_roadmap))
        .route("/api/billing/plans", get(get_billing_plans))
        .route("/api/billing/subscription", get(get_billing_subscription))
        .route("/api/billing/checkout-session", post(create_checkout_session))
        .route("/api/billing/webhook", post(billing_webhook))
        .route("/api/events", post(track_event))
        .route("/api/roadmaps/:id", put(update_roadmap)) // update roadmap title
        .route("/api/nodes/:id/references", get(get_node_references).post(add_node_reference)) // node references
        .route("/api/references/:id", delete(delete_node_reference)) // delete node reference
        .route("/api/share/:token/notes/:node_id", get(get_shared_note))
        .route("/api/share/:token/notes/:node_id/references", get(get_shared_node_references))
        .route("/api/nodes/:id", put(update_node).delete(delete_node))
        .route("/api/org/details", get(get_org_details).put(update_org_details))
        .route("/api/org/invite", post(create_org_invite))
        .layer(CorsLayer::permissive()).with_state(pool);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("Pathio Backend Pro started at http://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}
