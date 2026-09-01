use async_trait::async_trait;
use serde_json::json;

use crate::agent::tool::{OfficeTool, ToolArtifact, ToolContext, ToolResult};
use crate::models::NasConfig;

/// Let's Encrypt 新增根证书（Root YE + ISRG Root X2，2025 年新中间证书体系），
/// macOS 系统 CA bundle 尚未收录，懒猫微服 *.heiyu.space 证书链需要它。
const LE_NEW_ROOTS_PEM: &str = include_str!("../../../assets/le-new-roots.pem");

/// 读取用户全部启用的 WebDAV 数据源凭据，组装成随媒体请求传给 worker 的数组。
/// 用于本地 NAS 生图/生视频：worker 自动在局域网识别 NAS 并匹配凭据，不落盘到 spark。
pub async fn build_nas_credentials(user_id: &str) -> Option<serde_json::Value> {
    let sources = enabled_nas_sources(user_id).await;
    if sources.is_empty() {
        return None;
    }
    Some(serde_json::json!(sources.iter().map(|cfg| {
        serde_json::json!({
            "name": cfg.name,
            "base": cfg.base_url,
            "user": cfg.username,
            "password": cfg.password,
        })
    }).collect::<Vec<_>>()))
}

/// worker 中继地址与密钥：office 与媒体 worker 同机部署时的回环默认值，
/// 无需用户配置（spark 局域网 NAS 由 worker 自动识别）。
const WORKER_RELAY_URL: &str = "http://127.0.0.1:19095";
const WORKER_RELAY_KEY: &str = "media-worker-2026";

/// 自动访问策略：先直连 WebDAV（office 与数据源同网时最快），
/// 连不上时自动降级到局域网 worker 中继（数据面在 NAS↔spark 局域网内完成）。
fn relay_worker_url() -> String {
    std::env::var("WORKER_RELAY_URL").unwrap_or_else(|_| WORKER_RELAY_URL.to_string())
}
fn relay_worker_key() -> String {
    std::env::var("WORKER_RELAY_KEY").unwrap_or_else(|_| WORKER_RELAY_KEY.to_string())
}

fn relay_headers(cfg: &NasConfig) -> Vec<(String, String)> {
    vec![
        ("Authorization".into(), format!("Bearer {}", relay_worker_key())),
        ("X-NAS-Base".into(), cfg.base_url.clone()),
        ("X-NAS-User".into(), cfg.username.clone()),
        ("X-NAS-Pass".into(), cfg.password.clone()),
    ]
}

/// 字节版 worker 请求（音频等二进制：不经过 UTF-8 转换）
async fn worker_request_bytes(
    cfg: &NasConfig,
    method: &str,
    url: &str,
    body: Option<Vec<u8>>,
) -> anyhow::Result<(u16, Vec<u8>)> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(900))
        .build()?;
    let mut req = client
        .request(reqwest::Method::from_bytes(method.as_bytes())?, url)
        .header("Authorization", format!("Bearer {}", relay_worker_key()))
        .header("X-NAS-Base", cfg.base_url.clone())
        .header("X-NAS-User", cfg.username.clone())
        .header("X-NAS-Pass", cfg.password.clone());
    if let Some(b) = body {
        req = req.body(b);
    }
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await?.to_vec();
    Ok((status, bytes))
}

/// 经 worker 控制面发 HTTP 请求。
/// NAS 的 WebDAV 凭据来自 office 数据源配置（用户已在设置里填好），
/// 通过请求头传给 worker 逐次使用——凭据不落盘到 spark，用完即弃。
async fn worker_request(
    cfg: &NasConfig,
    method: &str,
    url: &str,
    body: Option<Vec<u8>>,
) -> anyhow::Result<(u16, String)> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(900))
        .build()?;
    let mut req = client
        .request(reqwest::Method::from_bytes(method.as_bytes())?, url)
        .header("Authorization", format!("Bearer {}", relay_worker_key()))
        .header("X-NAS-Base", cfg.base_url.clone())
        .header("X-NAS-User", cfg.username.clone())
        .header("X-NAS-Pass", cfg.password.clone());
    if let Some(b) = body {
        req = req.body(b);
    }
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    Ok((status, text))
}

/// 汇总当前用户全部启用的 WebDAV 数据源（旧单源 nas_config + 新多源 nas_configs，去重）
pub async fn enabled_nas_sources(user_id: &str) -> Vec<NasConfig> {
    let pool = crate::state::db_pool();
    let settings = match crate::db::settings_repo::find_by_user(&pool, user_id).await {
        Ok(Some(s)) => s,
        _ => return Vec::new(),
    };
    let mut out: Vec<NasConfig> = Vec::new();
    for cfg in settings.nas_configs.iter().chain(std::iter::once(&settings.nas_config)) {
        if !cfg.enabled || cfg.base_url.trim().is_empty() || cfg.username.trim().is_empty() {
            continue;
        }
        if out.iter().any(|c| c.base_url == cfg.base_url) {
            continue;
        }
        out.push(cfg.clone());
    }
    out
}

/// 二进制写入（音频等）：直连优先，失败自动降级 worker 中继。字节不经过 UTF-8 转换。
async fn dav_write_bytes(cfg: &NasConfig, rel_path: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let full_path = sanitize_path(rel_path)?;
    // 直连
    let url = join_url(&cfg.base_url, &full_path);
    if dav_http_request_bytes("PUT", &url, &cfg.username, &cfg.password, bytes)
        .map(|(s, _)| (200..300).contains(&s))
        .unwrap_or(false)
    {
        return Ok(());
    }
    // 中继
    let wurl = format!("{}/nas/put?path={}", relay_worker_url().trim_end_matches('/'), full_path);
    let (status, resp) = worker_request_bytes(cfg, "PUT", &wurl, Some(bytes.to_vec())).await?;
    if status != 200 {
        return Err(anyhow::anyhow!("worker 写入失败（HTTP {status}）: {}", String::from_utf8_lossy(&resp).chars().take(300).collect::<String>()));
    }
    Ok(())
}

/// 二进制读取（音频等）：直连优先，失败自动降级 worker 中继。
async fn dav_read_bytes(cfg: &NasConfig, rel_path: &str) -> anyhow::Result<Vec<u8>> {
    let full_path = sanitize_path(rel_path)?;
    let url = join_url(&cfg.base_url, &full_path);
    if let Ok((status, bytes)) = dav_http_request_bytes("GET", &url, &cfg.username, &cfg.password, b"") {
        if (200..300).contains(&status) {
            return Ok(bytes);
        }
    }
    let wurl = format!("{}/nas/get?path={}", relay_worker_url().trim_end_matches('/'), full_path);
    let (status, bytes) = worker_request_bytes(cfg, "GET", &wurl, None).await?;
    if status != 200 {
        return Err(anyhow::anyhow!("worker 读取失败（HTTP {status}）"));
    }
    Ok(bytes)
}

/// 供路由层使用：把二进制内容写入用户的第一个可用数据源（自动直连/中继路由）。
pub async fn write_file_for_user(user_id: &str, rel_path: &str, bytes: &[u8]) -> anyhow::Result<String> {
    let sources = enabled_nas_sources(user_id).await;
    if sources.is_empty() {
        return Err(anyhow::anyhow!("尚未配置 WebDAV 数据源"));
    }
    // 依次尝试每个源（直连失败自动降级中继由 dav_write 内部处理）
    let mut last_err = None;
    for cfg in &sources {
        match dav_write_bytes(cfg, rel_path, bytes).await {
            Ok(()) => return Ok(format!("{}:{rel_path}", if cfg.name.is_empty() { cfg.base_url.clone() } else { cfg.name.clone() })),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("WebDAV 写入失败")))
}

/// 供路由层使用：从用户数据源读取二进制文件（自动直连/中继路由）。
pub async fn read_file_for_user(user_id: &str, rel_path: &str) -> anyhow::Result<Vec<u8>> {
    let sources = enabled_nas_sources(user_id).await;
    if sources.is_empty() {
        return Err(anyhow::anyhow!("尚未配置 WebDAV 数据源"));
    }
    let mut last_err = None;
    for cfg in &sources {
        match dav_read_bytes(cfg, rel_path).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("WebDAV 读取失败")))
}

/// 供路由层使用：向局域网 worker 发通用控制面请求（流式会话等）。
pub async fn relay_stream(method: &str, path: &str, body: Option<serde_json::Value>) -> anyhow::Result<(u16, String)> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(900))
        .build()?;
    let mut req = client
        .request(reqwest::Method::from_bytes(method.as_bytes())?, format!("{}{}", relay_worker_url().trim_end_matches('/'), path))
        .header("Authorization", format!("Bearer {}", relay_worker_key()));
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((status, text))
}

/// 供路由层使用：把 WAV 字节交给局域网 worker 做本地转写（faster-whisper）。
/// 音频数据经 frp 控制通道传到 spark，不经过任何第三方服务。
pub async fn relay_transcribe(wav: &[u8]) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1200))
        .build()?;
    let resp = client
        .post(format!("{}/transcribe", relay_worker_url().trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", relay_worker_key()))
        .body(wav.to_vec())
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(anyhow::anyhow!("worker 转写失败（HTTP {status}）：{}", body.chars().take(300).collect::<String>()));
    }
    let j: serde_json::Value = serde_json::from_str(&body).map_err(|e| anyhow::anyhow!("worker 响应解析失败: {e}"))?;
    j.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("转写结果为空"))
}

/// 按名称或顺序取一个数据源（工具支持 nas_source 参数指定，缺省用第一个启用源）
async fn resolve_nas_source(ctx: &ToolContext, source_name: &str) -> anyhow::Result<NasConfig> {
    let sources = enabled_nas_sources(&ctx.user_id).await;
    if sources.is_empty() {
        return Err(anyhow::anyhow!(
            "尚未配置可用的 WebDAV 数据源，请先在「设置 → 数据源」中填写 WebDAV 地址与凭据"
        ));
    }
    if !source_name.is_empty() {
        if let Some(cfg) = sources.iter().find(|c| c.name == source_name) {
            return Ok(cfg.clone());
        }
        return Err(anyhow::anyhow!("未找到名为「{source_name}」的数据源"));
    }
    Ok(sources[0].clone())
}

/// 用 openssl 建立 TLS 连接（同步，跑在 spawn_blocking 里）。
/// 不依赖 security-framework（懒猫 NE 会拦截 security-framework 的 TLS → -9806 errSecIO），
/// 用 openssl 独立 TLS 栈 + 系统 CA + Let's Encrypt 新根证书。
fn connect_dav_tls(host: &str, port: u16) -> anyhow::Result<openssl::ssl::SslStream<std::net::TcpStream>> {
    use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
    use openssl::x509::store::X509StoreBuilder;

    let mut builder = SslConnector::builder(SslMethod::tls())?;
    builder.set_verify(SslVerifyMode::PEER);
    let mut store = X509StoreBuilder::new()?;
    store.set_default_paths()?;
    // 显式加载系统 CA（macOS /etc/ssl/cert.pem），vendored openssl 默认路径不含它
    for sys_ca in ["/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt", "/usr/local/etc/openssl/cert.pem"] {
        if std::path::Path::new(sys_ca).exists() {
            if let Ok(certs) = openssl::x509::X509::stack_from_pem(std::fs::read(sys_ca).unwrap_or_default().as_slice()) {
                for cert in certs {
                    let _ = store.add_cert(cert);
                }
            }
        }
    }
    // 追加 Let's Encrypt 新根（Root YE + ISRG Root X2），补系统 CA 的缺失
    let certs = openssl::x509::X509::stack_from_pem(LE_NEW_ROOTS_PEM.as_bytes())?;
    for cert in certs {
        let _ = store.add_cert(cert);
    }
    builder.set_verify_cert_store(store.build())?;
    let connector = builder.build();

    let stream = std::net::TcpStream::connect((host, port))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(30)))?;
    let ssl = connector.connect(host, stream)?;
    Ok(ssl)
}

/// 同步 WebDAV HTTP 请求（openssl TLS + 手写 HTTP/1.1）。
/// 返回 (status_code, response_body)。
fn dav_http_request(
    method: &str,
    url: &str,
    username: &str,
    password: &str,
    body: Option<&str>,
) -> anyhow::Result<(u16, String)> {
    use std::io::{Read, Write};

    // 解析 URL
    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("非法 URL（缺少协议）: {url}"))?;
    if scheme != "https" && scheme != "http" {
        return Err(anyhow::anyhow!("仅支持 http/https 协议: {scheme}"));
    }
    let (host, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".to_string()));
    let (host, port) = if let Some((h, p)) = host.split_once(':') {
        (h.to_string(), p.parse::<u16>()?)
    } else {
        (host.to_string(), if scheme == "https" { 443 } else { 80 })
    };

    // Basic Auth
    let auth = base64::encode(format!("{username}:{password}"));

    // 组装请求
    let body = body.unwrap_or("");
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Basic {auth}\r\nConnection: close\r\n"
    );
    if method == "PROPFIND" {
        req.push_str("Depth: 1\r\n");
    }
    if !body.is_empty() {
        req.push_str(&format!("Content-Type: application/xml\r\nContent-Length: {}\r\n", body.len()));
    } else {
        req.push_str("Content-Length: 0\r\n");
    }
    req.push_str("\r\n");
    req.push_str(body);

    // 连接：https 走 openssl TLS；http 明文（office 与数据源同局域网时直连）
    let mut resp = Vec::new();
    if scheme == "https" {
        let mut ssl = connect_dav_tls(&host, port)?;
        ssl.write_all(req.as_bytes())?;
        ssl.read_to_end(&mut resp)?;
    } else {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect((host.as_str(), port))?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(30)))?;
        stream.write_all(req.as_bytes())?;
        stream.read_to_end(&mut resp)?;
    }
    let resp_str = String::from_utf8_lossy(&resp).to_string();

    // 解析状态码
    let status_line = resp_str.lines().next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| anyhow::anyhow!("无法解析 HTTP 状态行: {status_line}"))?;

    // 分离 header 和 body
    let (header_part, body_part) = resp_str
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("响应缺少 header/body 分隔"))?;

    // 处理 Transfer-Encoding: chunked
    let headers = header_part.to_lowercase();
    let body = if headers.contains("transfer-encoding: chunked") {
        decode_chunked(body_part.as_bytes())
    } else {
        body_part.to_string()
    };

    Ok((status, body))
}

/// 字节版直连 WebDAV HTTP 请求（音频等二进制：body/响应均为原始字节）
fn dav_http_request_bytes(
    method: &str,
    url: &str,
    username: &str,
    password: &str,
    body: &[u8],
) -> anyhow::Result<(u16, Vec<u8>)> {
    use std::io::{Read, Write};

    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("非法 URL（缺少协议）: {url}"))?;
    if scheme != "https" && scheme != "http" {
        return Err(anyhow::anyhow!("仅支持 http/https 协议: {scheme}"));
    }
    let (host, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".to_string()));
    let (host, port) = if let Some((h, p)) = host.split_once(':') {
        (h.to_string(), p.parse::<u16>()?)
    } else {
        (host.to_string(), if scheme == "https" { 443 } else { 80 })
    };
    let auth = base64::encode(format!("{username}:{password}"));

    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Basic {auth}\r\nConnection: close\r\n"
    )
    .into_bytes();
    req.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    req.extend_from_slice(body);

    let mut resp = Vec::new();
    if scheme == "https" {
        let mut ssl = connect_dav_tls(&host, port)?;
        ssl.write_all(&req)?;
        ssl.read_to_end(&mut resp)?;
    } else {
        let mut stream = std::net::TcpStream::connect((host.as_str(), port))?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(60)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(60)))?;
        stream.write_all(&req)?;
        stream.read_to_end(&mut resp)?;
    }

    // 解析状态码与 body（字节安全）
    let sep = resp.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(resp.len());
    let head = String::from_utf8_lossy(&resp[..sep]).to_string();
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| anyhow::anyhow!("无法解析 HTTP 状态行: {}", head.chars().take(80).collect::<String>()))?;
    let body_bytes = if head.to_lowercase().contains("transfer-encoding: chunked") {
        decode_chunked_bytes(&resp[sep + 4..])
    } else {
        resp[sep + 4..].to_vec()
    };
    Ok((status, body_bytes))
}

fn decode_chunked_bytes(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let line_end = data[i..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .map(|p| i + p)
            .unwrap_or(data.len());
        let size_str = String::from_utf8_lossy(&data[i..line_end]);
        let size = usize::from_str_radix(size_str.trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        i = line_end + 2;
        if i + size + 2 <= data.len() {
            out.extend_from_slice(&data[i..i + size]);
            i += size + 2;
        } else {
            break;
        }
    }
    out
}

/// 解码 HTTP chunked 编码的 body
fn decode_chunked(data: &[u8]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        // 读 chunk size（十六进制，到 \r\n）
        let line_end = data[i..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .map(|p| i + p)
            .unwrap_or(data.len());
        let size_str = String::from_utf8_lossy(&data[i..line_end]);
        let size = usize::from_str_radix(size_str.trim(), 16).unwrap_or(0);
        if size == 0 {
            break; // 终止 chunk
        }
        i = line_end + 2; // 跳过 size 行
        out.extend_from_slice(&data[i..(i + size).min(data.len())]);
        i += size;
        // 跳过 chunk 后的 \r\n
        if i + 2 <= data.len() && data[i..i + 2] == *b"\r\n" {
            i += 2;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 异步包装：把同步 openssl 请求放到 spawn_blocking（避免阻塞 tokio 线程）。
async fn dav_request(
    method: &'static str,
    url: String,
    username: String,
    password: String,
    body: Option<String>,
) -> anyhow::Result<(u16, String)> {
    tokio::task::spawn_blocking(move || {
        dav_http_request(method, &url, &username, &password, body.as_deref())
    })
    .await
    .map_err(|e| anyhow::anyhow!("WebDAV 任务执行失败: {e}"))?
}

/// 拼接 WebDAV 路径（去掉末尾/开头的多余斜杠）
fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.is_empty() {
        base.to_string()
    } else {
        format!("{}/{}", base, path.trim_start_matches('/'))
    }
}

/// 清理用户传入的相对路径：去掉首尾斜杠，拒绝 `..` 穿越。
/// 路径相对 WebDAV 根（即该懒猫账号自己的文件空间根）。
fn sanitize_path(path: &str) -> anyhow::Result<String> {
    let rel = path.trim().trim_matches('/');
    // 路径穿越防护：不允许 .. 段
    for seg in rel.split('/') {
        if seg == ".." {
            return Err(anyhow::anyhow!("非法路径：不允许使用 '..'"));
        }
    }
    Ok(rel.to_string())
}

/// 测试 NAS（WebDAV）连接：对根目录发 PROPFIND，返回目录项数量。
/// 用于前端「测试连接」按钮。纯 HTTP(S) 请求，不挂载文件系统。
pub async fn test_nas_connection(cfg: &NasConfig) -> anyhow::Result<usize> {
    if !cfg.enabled || cfg.base_url.trim().is_empty() {
        return Err(anyhow::anyhow!("请先填写并启用 WebDAV 数据源"));
    }
    if cfg.username.trim().is_empty() {
        return Err(anyhow::anyhow!("请填写 WebDAV 用户名"));
    }
    if cfg.password.is_empty() {
        return Err(anyhow::anyhow!("请填写 WebDAV 密码"));
    }
    let files = dav_list(cfg, "").await?;
    Ok(files.len())
}

/// PROPFIND 列目录（Depth: 1），返回 (名称, 是否目录, 大小, 最后修改)
async fn dav_list(
    cfg: &NasConfig,
    rel_path: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let full_path = match sanitize_path(rel_path) { Ok(p) => p, Err(e) => return Err(e) };
    match dav_list_direct(cfg, &full_path).await {
        Ok(items) => Ok(items),
        Err(direct_err) => {
            tracing::debug!("NAS 直连失败（{direct_err:#}），自动降级 worker 中继");
            dav_list_relay(cfg, &full_path).await
        }
    }
}

async fn dav_list_direct(cfg: &NasConfig, full_path: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let url = join_url(&cfg.base_url, &full_path);
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:displayname/>
    <d:resourcetype/>
    <d:getcontentlength/>
    <d:getlastmodified/>
  </d:prop>
</d:propfind>"#;
    let (status, resp_body) = dav_request(
        "PROPFIND",
        url,
        cfg.username.clone(),
        cfg.password.clone(),
        Some(body.to_string()),
    )
    .await?;

    if !(200..300).contains(&status) {
        let detail = match status {
            401 => "用户名或密码错误（认证失败）".to_string(),
            403 => "没有访问权限（账号无该目录权限）".to_string(),
            404 => "路径不存在".to_string(),
            405 => "该操作不被允许（WebDAV 方法不支持）".to_string(),
            500..=599 => format!("服务器内部错误（{status}）"),
            _ => format!("HTTP {status}"),
        };
        let body_hint = if resp_body.trim().is_empty() { String::new() } else { format!(": {}", resp_body.chars().take(200).collect::<String>()) };
        return Err(anyhow::anyhow!("WebDAV 请求失败（{detail}）{body_hint}"));
    }

    parse_propfind_response(&resp_body)
}

async fn dav_list_relay(cfg: &NasConfig, path: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let url = format!("{}/nas/list?path={}", relay_worker_url().trim_end_matches('/'), path);
    let (status, body) = worker_request(cfg, "GET", &url, None).await?;
    if status != 200 {
        return Err(anyhow::anyhow!("worker 列目录失败（HTTP {status}）: {}", body.chars().take(300).collect::<String>()));
    }
    let j: serde_json::Value = serde_json::from_str(&body).map_err(|e| anyhow::anyhow!("worker 响应解析失败: {e}"))?;
    let items = j
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|it| json!({
            "name": it.get("name").and_then(|v| v.as_str()).unwrap_or(""),
            "href": it.get("name").and_then(|v| v.as_str()).unwrap_or(""),
            "is_dir": it.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false),
            "size": it.get("size").and_then(|v| v.as_i64()).unwrap_or(0),
            "modified": it.get("mtime").and_then(|v| v.as_str()).unwrap_or(""),
        }))
        .collect::<Vec<_>>();
    Ok(items)
}

/// 解析 PROPFIND 的 multistatus XML
fn parse_propfind_response(body: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut items = Vec::new();
    // 用简易 XML 解析：按 <d:response> 分段
    let doc = roxmltree::Document::parse(body).map_err(|e| anyhow::anyhow!("解析 WebDAV 响应失败: {e}"))?;

    for response in doc.descendants().filter(|n| n.has_tag_name("response")) {
        let mut href = String::new();
        let mut displayname = String::new();
        let mut is_collection = false;
        let mut size = 0i64;
        let mut lastmod = String::new();

        for node in response.descendants() {
            if node.has_tag_name("href") {
                if let Some(t) = node.text() {
                    href = t.to_string();
                }
            }
            if node.has_tag_name("displayname") {
                if let Some(t) = node.text() {
                    displayname = t.to_string();
                }
            }
            if node.has_tag_name("collection") {
                is_collection = true;
            }
            if node.has_tag_name("getcontentlength") {
                if let Some(t) = node.text() {
                    size = t.trim().parse().unwrap_or(0);
                }
            }
            if node.has_tag_name("getlastmodified") {
                if let Some(t) = node.text() {
                    lastmod = t.to_string();
                }
            }
        }

        if href.is_empty() || displayname.is_empty() {
            continue;
        }
        // 跳过当前目录本身
        let is_self = href.trim_end_matches('/') == displayname.trim_end_matches('/')
            || displayname == "."
            || displayname.is_empty();
        if is_self {
            continue;
        }

        items.push(json!({
            "name": displayname,
            "href": href,
            "is_dir": is_collection,
            "size": size,
            "modified": lastmod,
        }));
    }

    if items.is_empty() && body.contains("multistatus") {
        // 空目录（只有自身）也是合法的，返回空数组
        return Ok(items);
    }
    Ok(items)
}

/// GET 读取文件内容
async fn dav_read(cfg: &NasConfig, rel_path: &str) -> anyhow::Result<(String, String)> {
    let full_path = match sanitize_path(rel_path) { Ok(p) => p, Err(e) => return Err(e) };
    match dav_read_direct(cfg, &full_path).await {
        Ok(v) => Ok(v),
        Err(_direct_err) => {
            let url = format!("{}/nas/get?path={}", relay_worker_url().trim_end_matches('/'), full_path);
            let (status, body) = worker_request(cfg, "GET", &url, None).await?;
            if status != 200 {
                return Err(anyhow::anyhow!("worker 读取失败（HTTP {status}）: {}", body.chars().take(300).collect::<String>()));
            }
            Ok((body, "text/plain".to_string()))
        }
    }
}

async fn dav_read_direct(cfg: &NasConfig, full_path: &str) -> anyhow::Result<(String, String)> {
    let url = join_url(&cfg.base_url, &full_path);
    let (status, body) = dav_request(
        "GET",
        url,
        cfg.username.clone(),
        cfg.password.clone(),
        None,
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(anyhow::anyhow!("读取文件失败（HTTP {status}）"));
    }
    Ok((body, "text/plain".to_string()))
}

/// PUT 写入文件
async fn dav_write(cfg: &NasConfig, rel_path: &str, content: &str) -> anyhow::Result<()> {
    let full_path = match sanitize_path(rel_path) { Ok(p) => p, Err(e) => return Err(e) };
    match dav_write_direct(cfg, &full_path, content).await {
        Ok(()) => Ok(()),
        Err(_direct_err) => {
            let url = format!("{}/nas/put?path={}", relay_worker_url().trim_end_matches('/'), full_path);
            let (status, body) = worker_request(cfg, "PUT", &url, Some(content.as_bytes().to_vec())).await?;
            if status != 200 {
                return Err(anyhow::anyhow!("worker 写入失败（HTTP {status}）: {}", body.chars().take(300).collect::<String>()));
            }
            Ok(())
        }
    }
}

async fn dav_write_direct(cfg: &NasConfig, full_path: &str, content: &str) -> anyhow::Result<()> {
    let url = join_url(&cfg.base_url, &full_path);
    let (status, body) = dav_request(
        "PUT",
        url,
        cfg.username.clone(),
        cfg.password.clone(),
        Some(content.to_string()),
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(anyhow::anyhow!("写入文件失败（HTTP {status}）: {}", body.chars().take(200).collect::<String>()));
    }
    Ok(())
}

/// MKCOL 建目录
async fn dav_mkdir(cfg: &NasConfig, rel_path: &str) -> anyhow::Result<()> {
    let full_path = match sanitize_path(rel_path) { Ok(p) => p, Err(e) => return Err(e) };
    match dav_mkdir_direct(cfg, &full_path).await {
        Ok(()) => Ok(()),
        Err(_direct_err) => {
            let url = format!("{}/nas/mkdir", relay_worker_url().trim_end_matches('/'));
            let body = serde_json::json!({ "path": full_path }).to_string();
            let (status, resp_body) = worker_request(cfg, "POST", &url, Some(body.as_bytes().to_vec())).await?;
            if status != 200 {
                return Err(anyhow::anyhow!("worker 建目录失败（HTTP {status}）: {}", resp_body.chars().take(300).collect::<String>()));
            }
            Ok(())
        }
    }
}

async fn dav_mkdir_direct(cfg: &NasConfig, full_path: &str) -> anyhow::Result<()> {
    let url = join_url(&cfg.base_url, &full_path);
    let (status, body) = dav_request(
        "MKCOL",
        url,
        cfg.username.clone(),
        cfg.password.clone(),
        None,
    )
    .await?;
    // 201 Created / 405 已存在都算可接受
    if !(200..300).contains(&status) && status != 405 {
        return Err(anyhow::anyhow!("创建目录失败（HTTP {status}）: {}", body.chars().take(200).collect::<String>()));
    }
    Ok(())
}

/// 把 NAS 列表结果序列化为 markdown 表格文本
fn files_to_markdown(files: &[serde_json::Value]) -> String {
    if files.is_empty() {
        return "（空目录）".to_string();
    }
    let mut lines = vec!["| 名称 | 类型 | 大小 | 修改时间 |".to_string(), "| --- | --- | --- | --- |".to_string()];
    for f in files {
        let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let is_dir = f.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
        let kind = if is_dir { "📁 目录" } else { "📄 文件" };
        let size = f.get("size").and_then(|v| v.as_i64()).unwrap_or(0);
        let size_str = if is_dir { "-".to_string() } else { format!("{} B", size) };
        let modified = f.get("modified").and_then(|v| v.as_str()).unwrap_or("");
        lines.push(format!("| {} | {} | {} | {} |", name, kind, size_str, modified));
    }
    lines.join("\n")
}

/// NAS 列出目录工具
pub struct NasListTool;

#[async_trait]
impl OfficeTool for NasListTool {
    fn name(&self) -> &str {
        "nas_list"
    }

    fn description(&self) -> &str {
        "列出懒猫微服 NAS（WebDAV）指定目录的文件与子目录。path 为空时列根目录。凭据按用户隔离，只访问当前用户自己的 WebDAV 数据源。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "NAS 上的目录路径（相对访问根，如 /docs 或空字符串表示根目录）" },
                "source": { "type": "string", "description": "数据源名称（可选，多个数据源时指定；不传用第一个启用源）" }
            },
            "required": []
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let source = input.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let cfg = match resolve_nas_source(ctx, source).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };
        match dav_list(&cfg, path).await {
            Ok(files) => {
                let md = files_to_markdown(&files);
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: format!("NAS 目录列表{}", if path.is_empty() { "（根）".to_string() } else { format!("：{path}") }),
                    content: json!({ "type": "markdown", "markdown": md, "source": "nas" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "files": files, "path": path })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("NAS 目录列表（{}）：\n{md}", if path.is_empty() { "/" } else { path }),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => ToolResult::err(format!("列 NAS 目录失败: {e}")),
        }
    }
}

/// NAS 读取文件工具
pub struct NasReadTool;

#[async_trait]
impl OfficeTool for NasReadTool {
    fn name(&self) -> &str {
        "nas_read"
    }

    fn description(&self) -> &str {
        "读取懒猫微服 NAS（WebDAV）上指定文件的内容（文本类文件）。凭据按用户隔离。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "NAS 上的文件路径（相对访问根，如 /docs/方案.md）" },
                "source": { "type": "string", "description": "数据源名称（可选，多个数据源时指定；不传用第一个启用源）" }
            },
            "required": ["path"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("缺少 path 参数");
        }
        let source = input.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let cfg = match resolve_nas_source(ctx, source).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };
        match dav_read(&cfg, path).await {
            Ok((text, _ctype)) => {
                let preview = text.chars().take(20000).collect::<String>();
                let truncated = text.chars().count() > 20000;
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: format!("NAS 文件：{path}"),
                    content: json!({ "type": "markdown", "markdown": preview, "source": "nas" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "path": path, "truncated": truncated })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("NAS 文件内容（{path}）：\n{preview}{}", if truncated { "\n…（内容过长已截断）" } else { "" }),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => ToolResult::err(format!("读 NAS 文件失败: {e}")),
        }
    }
}

/// NAS 写入文件工具
pub struct NasWriteTool;

#[async_trait]
impl OfficeTool for NasWriteTool {
    fn name(&self) -> &str {
        "nas_write"
    }

    fn description(&self) -> &str {
        "把内容写入懒猫微服 NAS（WebDAV）上的指定文件（覆盖写）。凭据按用户隔离。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "NAS 上的文件路径（相对访问根，如 /docs/纪要.md）" },
                "content": { "type": "string", "description": "要写入的文件内容" }
            },
            "required": ["path", "content"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("缺少 path 参数");
        }
        let source = input.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let cfg = match resolve_nas_source(ctx, source).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };
        match dav_write(&cfg, path, content).await {
            Ok(()) => ToolResult {
                success: true,
                data: Some(json!({ "path": path, "written": content.chars().count() })),
                error: None,
                artifacts: None,
                observation: format!("已写入 NAS 文件：{path}（{} 字符）", content.chars().count()),
                needs_auth: None,
                continue_loop: None,
            },
            Err(e) => ToolResult::err(format!("写 NAS 文件失败: {e}")),
        }
    }
}

/// NAS 建目录工具
pub struct NasMkdirTool;

#[async_trait]
impl OfficeTool for NasMkdirTool {
    fn name(&self) -> &str {
        "nas_mkdir"
    }

    fn description(&self) -> &str {
        "在懒猫微服 NAS（WebDAV）上创建目录。凭据按用户隔离。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "要创建的目录路径（相对访问根，如 /docs/2026）" }
            },
            "required": ["path"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("缺少 path 参数");
        }
        let source = input.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let cfg = match resolve_nas_source(ctx, source).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };
        match dav_mkdir(&cfg, path).await {
            Ok(()) => ToolResult {
                success: true,
                data: Some(json!({ "path": path })),
                error: None,
                artifacts: None,
                observation: format!("已在 NAS 创建目录：{path}"),
                needs_auth: None,
                continue_loop: None,
            },
            Err(e) => ToolResult::err(format!("建 NAS 目录失败: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_path_trims_and_blocks_traversal() {
        // 空路径 → 空
        assert_eq!(sanitize_path("").unwrap(), "");
        // 去首尾斜杠
        assert_eq!(sanitize_path("/docs/").unwrap(), "docs");
        assert_eq!(sanitize_path("docs").unwrap(), "docs");
        assert_eq!(sanitize_path("/a/b/c").unwrap(), "a/b/c");
        // 拒绝 .. 路径穿越
        assert!(sanitize_path("../bob").is_err());
        assert!(sanitize_path("a/../../b").is_err());
        assert!(sanitize_path("..").is_err());
        // 正常路径不受影响
        assert!(sanitize_path("a/b/c").is_ok());
    }

    #[test]
    fn join_url_trims_slashes() {
        assert_eq!(join_url("https://x.heiyu.space/dav", ""), "https://x.heiyu.space/dav");
        assert_eq!(join_url("https://x.heiyu.space/dav", "users/alice"), "https://x.heiyu.space/dav/users/alice");
        assert_eq!(join_url("https://x.heiyu.space/dav/", "/a/b"), "https://x.heiyu.space/dav/a/b");
    }
}
