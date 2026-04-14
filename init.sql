-- 1. 用户表 (Users)
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    nickname VARCHAR(50),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- 2. 组织表 (Organizations)
CREATE TABLE organizations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL,
    owner_id UUID REFERENCES users(id), -- 组织创建者
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- 3. 组织成员关联表 (Org_Members)
CREATE TABLE org_members (
    org_id UUID REFERENCES organizations(id) ON DELETE CASCADE,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    role VARCHAR(20) DEFAULT 'member', -- admin, editor, member(只读)
    PRIMARY KEY (org_id, user_id)
);

-- 4. 路线图空间 (Roadmaps)
CREATE TABLE roadmaps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID REFERENCES organizations(id) ON DELETE CASCADE,
    title VARCHAR(255) NOT NULL,
    description TEXT,
    share_token VARCHAR(100) UNIQUE, -- 用于只读分享的唯一链接参数
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- 5. 画布节点 (Nodes)
CREATE TABLE nodes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    roadmap_id UUID REFERENCES roadmaps(id) ON DELETE CASCADE,
    title VARCHAR(255) NOT NULL,
    status VARCHAR(20) DEFAULT 'todo', -- todo, doing, done
    assignee_id UUID REFERENCES users(id) ON DELETE SET NULL, -- 任务分配给谁
    pos_x FLOAT NOT NULL, -- 画布上的 X 坐标
    pos_y FLOAT NOT NULL, -- 画布上的 Y 坐标
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- 6. 画布连线 (Edges)
CREATE TABLE edges (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    roadmap_id UUID REFERENCES roadmaps(id) ON DELETE CASCADE,
    source_node_id UUID REFERENCES nodes(id) ON DELETE CASCADE,
    target_node_id UUID REFERENCES nodes(id) ON DELETE CASCADE,
    UNIQUE(source_node_id, target_node_id) -- 防止重复连线
);

-- 7. 笔记表 (Notes)
CREATE TABLE notes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    node_id UUID UNIQUE REFERENCES nodes(id) ON DELETE CASCADE, -- 一对一：一个节点对应一篇主笔记
    content JSONB, -- 推荐存 JSONB，方便适配类似 Notion 的 Block 富文本数据
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE nodes ALTER COLUMN roadmap_id DROP NOT NULL;


-- 1. 确保 nickname 不能为空 (请确保表中现有数据 nickname 都有值，否则会报错)
ALTER TABLE users ALTER COLUMN nickname SET NOT NULL;

-- 2. 确保 nickname 全局唯一（因为要用来登录）
ALTER TABLE users ADD CONSTRAINT users_nickname_unique UNIQUE (nickname);

-- 修改组织表，增加版本标记
ALTER TABLE organizations ADD COLUMN plan_type VARCHAR(20) DEFAULT 'free'; -- 'free', 'team'

-- 邀请链接表
CREATE TABLE invitations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    inviter_id UUID NOT NULL REFERENCES users(id),
    code VARCHAR(20) UNIQUE NOT NULL,
    is_used BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);


-- 创建节点参考引用表
CREATE TABLE node_references (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    node_id UUID NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Commercialization loop: billing + entitlement + analytics
ALTER TABLE organizations ADD COLUMN IF NOT EXISTS billing_status VARCHAR(20) DEFAULT 'inactive';
ALTER TABLE organizations ADD COLUMN IF NOT EXISTS current_period_end TIMESTAMP WITH TIME ZONE;
ALTER TABLE organizations ADD COLUMN IF NOT EXISTS billing_market VARCHAR(20) DEFAULT 'cn';

CREATE TABLE IF NOT EXISTS plan_entitlements (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    plan_type VARCHAR(20) NOT NULL,
    market VARCHAR(20) NOT NULL,
    currency VARCHAR(10) NOT NULL,
    price_cents INTEGER NOT NULL DEFAULT 0,
    billing_interval VARCHAR(20) NOT NULL DEFAULT 'month',
    max_roadmaps BIGINT,
    max_nodes_per_org BIGINT,
    max_members_per_org BIGINT,
    can_public_share BOOLEAN NOT NULL DEFAULT TRUE,
    priority_support BOOLEAN NOT NULL DEFAULT FALSE,
    sso_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    audit_log_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    private_deployment BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(plan_type, market)
);

INSERT INTO plan_entitlements (
    plan_type, market, currency, price_cents, billing_interval,
    max_roadmaps, max_nodes_per_org, max_members_per_org,
    can_public_share, priority_support, sso_enabled, audit_log_enabled, private_deployment
) VALUES
    ('free', 'cn', 'CNY', 0, 'month', 3, 50, 2, TRUE, FALSE, FALSE, FALSE, FALSE),
    ('team', 'cn', 'CNY', 3000, 'month', NULL, NULL, NULL, TRUE, TRUE, FALSE, FALSE, FALSE),
    ('enterprise', 'cn', 'CNY', 0, 'month', NULL, NULL, NULL, TRUE, TRUE, TRUE, TRUE, TRUE),
    ('free', 'global', 'USD', 0, 'month', 3, 50, 2, TRUE, FALSE, FALSE, FALSE, FALSE),
    ('team', 'global', 'USD', 900, 'month', NULL, NULL, NULL, TRUE, TRUE, FALSE, FALSE, FALSE),
    ('enterprise', 'global', 'USD', 0, 'month', NULL, NULL, NULL, TRUE, TRUE, TRUE, TRUE, TRUE)
ON CONFLICT (plan_type, market) DO UPDATE
SET currency = EXCLUDED.currency,
    price_cents = EXCLUDED.price_cents,
    billing_interval = EXCLUDED.billing_interval,
    max_roadmaps = EXCLUDED.max_roadmaps,
    max_nodes_per_org = EXCLUDED.max_nodes_per_org,
    max_members_per_org = EXCLUDED.max_members_per_org,
    can_public_share = EXCLUDED.can_public_share,
    priority_support = EXCLUDED.priority_support,
    sso_enabled = EXCLUDED.sso_enabled,
    audit_log_enabled = EXCLUDED.audit_log_enabled,
    private_deployment = EXCLUDED.private_deployment;

CREATE TABLE IF NOT EXISTS billing_checkout_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    plan_type VARCHAR(20) NOT NULL,
    market VARCHAR(20) NOT NULL,
    currency VARCHAR(10) NOT NULL,
    seats INTEGER NOT NULL DEFAULT 1,
    amount_cents INTEGER NOT NULL DEFAULT 0,
    provider VARCHAR(40) NOT NULL DEFAULT 'mock_gateway',
    external_session_id VARCHAR(64) NOT NULL UNIQUE,
    checkout_url TEXT NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'pending',
    provider_event_id VARCHAR(128),
    raw_payload JSONB,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS product_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(64) NOT NULL,
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    properties JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_product_events_name_time ON product_events(name, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_product_events_org_time ON product_events(org_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_checkout_sessions_org_time ON billing_checkout_sessions(org_id, created_at DESC);
