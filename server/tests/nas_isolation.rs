//! NAS（懒猫微服 WebDAV）per-user 凭据隔离测试
//! 验证：每个用户单独保存自己的 NAS 凭据，多用户挂载同一 NAS 互不冲突。

use std::sync::atomic::{AtomicU32, Ordering};
use walioffice::db::{settings_repo, user_repo};
use walioffice::models::NasConfig;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// 在模块加载时确保环境变量（config() 单例首次加载需要）
static ENV_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
fn ensure_env() {
    ENV_INIT.get_or_init(|| {
        if std::env::var("AIPPT_JWT_SECRET").is_err() {
            std::env::set_var("AIPPT_JWT_SECRET", "test-secret");
        }
    });
}

async fn make_pool() -> walioffice::db::DbPool {
    ensure_env();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join("walioffice-nas-test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("t{pid}_{n}.db"));
    let url = format!("sqlite://{}?mode=rwc", file.display());
    walioffice::db::init_pool(&url, "", 2).await.expect("init pool")
}

async fn create_user(pool: &walioffice::db::DbPool, name: &str) -> String {
    user_repo::create(pool, name, None, "pass123")
        .await
        .expect("create user")
        .id
}

/// 从最小 JSON 构造 AppSettings（不触发 config 加载，避免环境变量依赖）
fn minimal_settings() -> walioffice::models::AppSettings {
    serde_json::from_value(serde_json::json!({
        "llm_profiles": [{
            "id": "default", "name": "默认", "base_url": "http://x", "api_keys": [],
            "models": ["m"], "default_model": "m", "has_api_key": false
        }],
        "active_profile_id": "default",
        "default_model": "m",
        "active_model": "m",
        "basic": { "app_name": "W", "workspace_title": "W", "brand_tagline": "", "default_theme": "default" },
        "mcp_servers": [],
        "search_providers": { "tavily_api_key": "", "brave_api_key": "", "kimi_api_key": "", "provider": "auto" },
        "feishu_token": {},
        "nas_config": {},
        "updated_at": "2026-01-01T00:00:00Z"
    }))
    .expect("parse minimal settings")
}

#[tokio::test]
async fn nas_credentials_isolated_per_user() {
    let pool = make_pool().await;

    // 用户 A 和 B，各自配置不同的 NAS 凭据（挂载同一个 NAS 但用不同账号）
    let user_a = create_user(&pool, "alice").await;
    let user_b = create_user(&pool, "bob").await;

    let mut settings_a = minimal_settings();
    settings_a.nas_config = NasConfig {
        name: "A的资料库".into(),
        base_url: "https://roboterra.heiyu.space/dav".into(),
        username: "alice".into(),
        password: "alice-pass".into(),
        enabled: true,
    };
    settings_repo::save_for_user(&pool, &user_a, &settings_a).await.expect("save A");

    let mut settings_b = minimal_settings();
    settings_b.nas_config = NasConfig {
        name: "B的资料库".into(),
        base_url: "https://roboterra.heiyu.space/dav".into(),
        username: "bob".into(),
        password: "bob-pass".into(),
        enabled: true,
    };
    settings_repo::save_for_user(&pool, &user_b, &settings_b).await.expect("save B");

    // 各自读回，互不串
    let read_a = settings_repo::find_by_user(&pool, &user_a).await.expect("read A").expect("exists A");
    let read_b = settings_repo::find_by_user(&pool, &user_b).await.expect("read B").expect("exists B");

    assert_eq!(read_a.nas_config.username, "alice");
    assert_eq!(read_a.nas_config.password, "alice-pass");
    assert_eq!(read_a.nas_config.name, "A的资料库");

    assert_eq!(read_b.nas_config.username, "bob");
    assert_eq!(read_b.nas_config.password, "bob-pass");
    assert_eq!(read_b.nas_config.name, "B的资料库");

    // 同一个 NAS 地址（机器全局），但凭据不同 → 懒猫微服按账号隔离文件空间
    assert_eq!(read_a.nas_config.base_url, read_b.nas_config.base_url);
    assert_ne!(read_a.nas_config.username, read_b.nas_config.username);
}

#[tokio::test]
async fn nas_default_disabled_when_not_configured() {
    let pool = make_pool().await;
    let user = create_user(&pool, "carol").await;

    // 未配置 NAS 时，find_by_user 返回 None（尚未保存过 settings）
    let read = settings_repo::find_by_user(&pool, &user).await.expect("read");
    match read {
        Some(s) => {
            // 若存在 settings，nas_config 应为默认禁用
            assert!(!s.nas_config.enabled);
            assert!(s.nas_config.base_url.is_empty());
        }
        None => {
            // 未保存过 settings，符合预期（default_settings 在 get_settings 时才兜底）
        }
    }
}
