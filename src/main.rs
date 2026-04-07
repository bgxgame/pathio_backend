use axum::{
    extract::{State, Path, FromRequestParts, Query},
    http::StatusCode,
    routing::{get, put, post},
    Json, Router,
};
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, postgres::PgPoolOptions};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use uuid::Uuid;
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{encode, Header, EncodingKey, decode, DecodingKey, Validation};

// ==========================================
// 1. 数据模型定义
// ==========================================

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid, // User ID
    pub exp: usize,
}

#[derive(Deserialize)]
pub struct AuthReq {
    pub username: String,
    pub email: Option<String>,
    pub password: String,
    pub invite_code: Option<String>, // 新增：注册时支持邀请码
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

#[derive(Deserialize)]
pub struct UpdateNoteReq {
    pub content: serde_json::Value,
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

// 组织详情模型
#[derive(Serialize)]
pub struct OrgDetails {
    pub name: String,
    pub plan_type: String,
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

// JWT 提取器
#[axum::async_trait]
impl<S> FromRequestParts<S> for Claims
where S: Send + Sync,
{
    type Rejection = (StatusCode, String);
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts.headers.get("Authorization").and_then(|h| h.to_str().ok()).ok_or((StatusCode::UNAUTHORIZED, "未登录".to_string()))?;
        if !auth_header.starts_with("Bearer ") { return Err((StatusCode::UNAUTHORIZED, "Token格式错误".to_string())); }
        let token = &auth_header[7..];
        let token_data = decode::<Claims>(token, &DecodingKey::from_secret("secret".as_ref()), &Validation::default())
            .map_err(|_| (StatusCode::UNAUTHORIZED, "会话过期".to_string()))?;
        Ok(token_data.claims)
    }
}

// ==========================================
// 2. 认证逻辑 (包含邀请码处理)
// ==========================================

async fn register(State(pool): State<PgPool>, Json(payload): Json<AuthReq>) -> Result<StatusCode, (StatusCode, String)> {
    let email = payload.email.ok_or((StatusCode::BAD_REQUEST, "邮箱必填".to_string()))?;
    let hashed = hash(payload.password, DEFAULT_COST).unwrap();
    let mut tx = pool.begin().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 1. 创建用户
    let user_id: Uuid = sqlx::query_scalar("INSERT INTO users (nickname, email, password_hash) VALUES ($1, $2, $3) RETURNING id")
        .bind(payload.username).bind(email).bind(hashed).fetch_one(&mut *tx).await
        .map_err(|_| (StatusCode::BAD_REQUEST, "用户名或邮箱占用".to_string()))?;

    // 2. 检查是否有邀请码
    if let Some(code) = payload.invite_code {
        let org_id: Option<Uuid> = sqlx::query_scalar("SELECT org_id FROM invitations WHERE code = $1 AND is_used = FALSE")
            .bind(&code).fetch_optional(&mut *tx).await.unwrap();
        
        if let Some(oid) = org_id {
            // 加入现有组织
            sqlx::query("INSERT INTO org_members (org_id, user_id, role) VALUES ($1, $2, 'member')")
                .bind(oid).bind(user_id).execute(&mut *tx).await.unwrap();
            // 标记邀请码已使用（可选逻辑：此处标记为已用）
            sqlx::query("UPDATE invitations SET is_used = TRUE WHERE code = $1").bind(&code).execute(&mut *tx).await.unwrap();
        } else {
            return Err((StatusCode::BAD_REQUEST, "邀请码无效或已过期".to_string()));
        }
    } else {
        // 无邀请码：创建新组织
        let org_id: Uuid = sqlx::query_scalar("INSERT INTO organizations (name, owner_id, plan_type) VALUES ($1, $2, 'free') RETURNING id")
            .bind("我的默认空间").bind(user_id).fetch_one(&mut *tx).await.unwrap();
        
        // 自己也是管理员
        sqlx::query("INSERT INTO org_members (org_id, user_id, role) VALUES ($1, $2, 'admin')")
            .bind(org_id).bind(user_id).execute(&mut *tx).await.unwrap();

        // 创建初始路线图
        sqlx::query("INSERT INTO roadmaps (org_id, title, share_token) VALUES ($1, $2, $3)")
            .bind(org_id).bind("我的首个研究路径").bind(Uuid::new_v4().to_string()[..8].to_string()).execute(&mut *tx).await.unwrap();
    }

    tx.commit().await.unwrap();
    Ok(StatusCode::CREATED)
}

async fn login(State(pool): State<PgPool>, Json(payload): Json<AuthReq>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (id, hash_val): (Uuid, String) = sqlx::query_as("SELECT id, password_hash FROM users WHERE nickname = $1").bind(payload.username).fetch_optional(&pool).await.unwrap()
        .ok_or((StatusCode::UNAUTHORIZED, "用户不存在".to_string()))?;
    if verify(payload.password, &hash_val).unwrap() {
        let claims = Claims { sub: id, exp: 10000000000 }; 
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret("secret".as_ref())).unwrap();
        Ok(Json(serde_json::json!({ "token": token })))
    } else { Err((StatusCode::UNAUTHORIZED, "密码错误".to_string())) }
}

// ==========================================
// 3. 管理端业务 (Organization Management)
// ==========================================

async fn get_org_details(claims: Claims, State(pool): State<PgPool>) -> Result<Json<OrgDetails>, (StatusCode, String)> {
    // 1. 获取组织基本信息
    let org: (String, String, Uuid) = sqlx::query_as("
        SELECT o.name, o.plan_type, o.id FROM organizations o 
        JOIN org_members om ON o.id = om.org_id 
        WHERE om.user_id = $1 LIMIT 1
    ").bind(claims.sub).fetch_one(&pool).await.map_err(|_| (StatusCode::NOT_FOUND, "找不到组织".to_string()))?;

    // 2. 获取所有成员
    let members = sqlx::query_as::<_, OrgMemberInfo>("
        SELECT u.id, u.nickname, u.email, om.role, u.created_at FROM users u
        JOIN org_members om ON u.id = om.user_id
        WHERE om.org_id = $1
    ").bind(org.2).fetch_all(&pool).await.unwrap();

    Ok(Json(OrgDetails { name: org.0, plan_type: org.1, members }))
}

async fn create_org_invite(claims: Claims, State(pool): State<PgPool>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let org_id: Uuid = sqlx::query_scalar("SELECT org_id FROM org_members WHERE user_id = $1 AND role = 'admin'")
        .bind(claims.sub).fetch_one(&pool).await.map_err(|_| (StatusCode::FORBIDDEN, "仅管理员可邀请".to_string()))?;
    
    let code = Uuid::new_v4().to_string()[..6].to_uppercase();
    sqlx::query("INSERT INTO invitations (org_id, inviter_id, code) VALUES ($1, $2, $3)")
        .bind(org_id).bind(claims.sub).bind(&code).execute(&pool).await.unwrap();
    
    Ok(Json(serde_json::json!({ "code": code })))
}

// ==========================================
// 4. 业务逻辑 (Roadmap & Nodes)
// ==========================================

async fn create_roadmap(claims: Claims, State(pool): State<PgPool>, Json(payload): Json<serde_json::Value>) -> Result<Json<Roadmap>, (StatusCode, String)> {
    let org: (Uuid, String) = sqlx::query_as("SELECT o.id, o.plan_type FROM organizations o JOIN org_members om ON o.id = om.org_id WHERE om.user_id = $1 LIMIT 1")
        .bind(claims.sub).fetch_one(&pool).await.unwrap();

    if org.1 == "free" {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roadmaps WHERE org_id = $1").bind(org.0).fetch_one(&pool).await.unwrap();
        if count >= 1 { return Err((StatusCode::FORBIDDEN, "免费版限1个空间，请升级团队版".to_string())); }
    }

    let title = payload["title"].as_str().unwrap_or("未命名路线图");
    let res = sqlx::query_as::<_, Roadmap>("INSERT INTO roadmaps (org_id, title, share_token) VALUES ($1, $2, $3) RETURNING id, title, share_token")
        .bind(org.0).bind(title).bind(Uuid::new_v4().to_string()[..8].to_string()).fetch_one(&pool).await.unwrap();
    Ok(Json(res))
}

async fn get_roadmaps(claims: Claims, State(pool): State<PgPool>) -> Result<Json<Vec<Roadmap>>, (StatusCode, String)> {
    let query = "SELECT r.id, r.title, r.share_token FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $1 ORDER BY r.created_at DESC";
    let res = sqlx::query_as::<_, Roadmap>(query).bind(claims.sub).fetch_all(&pool).await.unwrap();
    Ok(Json(res))
}

async fn get_all_nodes(claims: Claims, Query(q): Query<RoadmapQuery>, State(pool): State<PgPool>) -> Result<Json<Vec<Node>>, (StatusCode, String)> {
    let query = "SELECT n.* FROM nodes n JOIN roadmaps r ON n.roadmap_id = r.id JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $1 AND r.id = $2";
    let res = sqlx::query_as::<_, Node>(query).bind(claims.sub).bind(q.roadmap_id).fetch_all(&pool).await.unwrap();
    Ok(Json(res))
}

async fn create_node(claims: Claims, State(pool): State<PgPool>, Json(payload): Json<CreateNodeReq>) -> Result<(StatusCode, Json<Node>), (StatusCode, String)> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE r.id = $1 AND om.user_id = $2)")
        .bind(payload.roadmap_id).bind(claims.sub).fetch_one(&pool).await.unwrap();
    if !exists { return Err((StatusCode::FORBIDDEN, "无权访问".to_string())); }

    let res = sqlx::query_as::<_, Node>("INSERT INTO nodes (roadmap_id, title, pos_x, pos_y) VALUES ($1, $2, $3, $4) RETURNING *")
        .bind(payload.roadmap_id).bind(payload.title).bind(payload.pos_x).bind(payload.pos_y).fetch_one(&pool).await.unwrap();
    Ok((StatusCode::CREATED, Json(res)))
}

async fn update_node(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>, Json(payload): Json<UpdateNodeReq>) -> Result<StatusCode, (StatusCode, String)> {
    let query = "UPDATE nodes SET title = COALESCE($1, title), status = COALESCE($2, status) WHERE id = $3 AND roadmap_id IN (SELECT r.id FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $4)";
    sqlx::query(query).bind(payload.title).bind(payload.status).bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    Ok(StatusCode::OK)
}

async fn update_node_position(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>, Json(payload): Json<UpdateNodePosReq>) -> Result<StatusCode, (StatusCode, String)> {
    let query = "UPDATE nodes SET pos_x = $1, pos_y = $2 WHERE id = $3 AND roadmap_id IN (SELECT r.id FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $4)";
    let res = sqlx::query(query).bind(payload.pos_x).bind(payload.pos_y).bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    if res.rows_affected() > 0 { Ok(StatusCode::OK) } else { Err((StatusCode::FORBIDDEN, "操作失败".to_string())) }
}

async fn delete_node(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>) -> Result<StatusCode, (StatusCode, String)> {
    let query = "DELETE FROM nodes WHERE id = $1 AND roadmap_id IN (SELECT r.id FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $2)";
    sqlx::query(query).bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    Ok(StatusCode::OK)
}

async fn get_all_edges(claims: Claims, Query(q): Query<RoadmapQuery>, State(pool): State<PgPool>) -> Result<Json<Vec<Edge>>, (StatusCode, String)> {
    let query = "SELECT e.* FROM edges e JOIN roadmaps r ON e.roadmap_id = r.id JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $1 AND r.id = $2";
    let res = sqlx::query_as::<_, Edge>(query).bind(claims.sub).bind(q.roadmap_id).fetch_all(&pool).await.unwrap();
    Ok(Json(res))
}

async fn create_edge(claims: Claims, State(pool): State<PgPool>, Json(payload): Json<CreateEdgeReq>) -> Result<StatusCode, (StatusCode, String)> {
    let query = "INSERT INTO edges (roadmap_id, source_node_id, target_node_id) SELECT $1, $2, $3 WHERE EXISTS (SELECT 1 FROM roadmaps r JOIN org_members om ON r.org_id = om.org_id WHERE r.id = $1 AND om.user_id = $4) ON CONFLICT DO NOTHING";
    sqlx::query(query).bind(payload.roadmap_id).bind(payload.source).bind(payload.target).bind(claims.sub).execute(&pool).await.unwrap();
    Ok(StatusCode::CREATED)
}

async fn get_node_note(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>) -> Result<Json<Note>, (StatusCode, String)> {
    let query = "INSERT INTO notes (node_id, content) SELECT $1, '{\"content\":[]}' WHERE EXISTS (SELECT 1 FROM nodes n JOIN roadmaps r ON n.roadmap_id = r.id JOIN org_members om ON r.org_id = om.org_id WHERE n.id = $1 AND om.user_id = $2) ON CONFLICT (node_id) DO UPDATE SET node_id = EXCLUDED.node_id RETURNING node_id, content";
    let res = sqlx::query_as::<_, Note>(query).bind(id).bind(claims.sub).fetch_one(&pool).await.map_err(|_| (StatusCode::FORBIDDEN, "无权访问".to_string()))?;
    Ok(Json(res))
}

async fn update_node_note(claims: Claims, Path(id): Path<Uuid>, State(pool): State<PgPool>, Json(payload): Json<UpdateNoteReq>) -> Result<StatusCode, (StatusCode, String)> {
    let query = "UPDATE notes SET content = $1, updated_at = CURRENT_TIMESTAMP WHERE node_id = $2 AND node_id IN (SELECT n.id FROM nodes n JOIN roadmaps r ON n.roadmap_id = r.id JOIN org_members om ON r.org_id = om.org_id WHERE om.user_id = $3)";
    sqlx::query(query).bind(payload.content).bind(id).bind(claims.sub).execute(&pool).await.unwrap();
    Ok(StatusCode::OK)
}

async fn get_shared_note(Path((token, node_id)): Path<(String, Uuid)>, State(pool): State<PgPool>) -> Result<Json<Note>, (StatusCode, String)> {
    let query = "SELECT n.node_id, n.content FROM notes n JOIN nodes nd ON n.node_id = nd.id JOIN roadmaps r ON nd.roadmap_id = r.id WHERE r.share_token = $1 AND n.node_id = $2";
    match sqlx::query_as::<_, Note>(query).bind(token).bind(node_id).fetch_one(&pool).await {
        Ok(note) => Ok(Json(note)),
        Err(_) => Err((StatusCode::FORBIDDEN, "无权访问".to_string())),
    }
}

async fn get_shared_roadmap(Path(token): Path<String>, State(pool): State<PgPool>) -> Result<Json<ShareData>, (StatusCode, String)> {
    let roadmap = sqlx::query_as::<_, Roadmap>("SELECT id, title, share_token FROM roadmaps WHERE share_token = $1").bind(&token).fetch_optional(&pool).await.unwrap().ok_or((StatusCode::NOT_FOUND, "无效".to_string()))?;
    let nodes = sqlx::query_as::<_, Node>("SELECT * FROM nodes WHERE roadmap_id = $1").bind(roadmap.id).fetch_all(&pool).await.unwrap();
    let edges = sqlx::query_as::<_, Edge>("SELECT * FROM edges WHERE roadmap_id = $1").bind(roadmap.id).fetch_all(&pool).await.unwrap();
    Ok(Json(ShareData { roadmap_title: roadmap.title, nodes, edges }))
}

async fn health_check() -> &'static str { "Pathio API Running 🚀" }

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
        .route("/api/share/:token/notes/:node_id", get(get_shared_note))
        .route("/api/nodes/:id", put(update_node).delete(delete_node))
        .route("/api/org/details", get(get_org_details))
        .route("/api/org/invite", post(create_org_invite))
        .layer(CorsLayer::permissive()).with_state(pool);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("🚀 Pathio Backend Pro started at http://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}