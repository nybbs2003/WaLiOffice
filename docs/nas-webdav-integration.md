# 懒猫微服 NAS 数据源集成（WebDAV，不依赖文件系统挂载）

> 调研日期：2026-08-28 · 实现落地：2026-09-01
> 目标：WaLiOffice（Moe Office）集成懒猫微服 NAS 作为数据源，**不依赖部署环境挂载 NAS 到文件系统**。

## 实现状态（2026-09-01 已落地）

本方案已从调研落地为生产实现，核心能力见 `server/src/agent/tools/nas_tools.rs`：

- **WebDAV 多数据源**：可配置任意多个 WebDAV 服务（设置界面列表管理）。
- **降级链**：office 部署公网时直连失败自动降级 worker 中继（局域网 spark 同网段 NAS 自动识别）。
- **凭据安全**：凭据随请求传入 worker，不落盘 spark。
- **生成集成**：生图/生视频工具的 `nas_refs` 引用 + `nas_out`/`nas_out_source` 指定存放路径与目的源；来源/目的存储解构（局域网/远端四种组合任意）。
- **二进制安全**：WebDAV 读写走字节通道（WAV 等二进制不再经 UTF-8 损坏）。
- 明文 http 支持（同局域网场景）。

## 结论摘要

懒猫微服（懒猫网盘）原生支持 **WebDAV 协议**，提供公网域名（`*.heiyu.space`），443 端口可直接访问。**用 WebDAV 即可实现「不挂载文件系统」的 NAS 文件访问**。

## 懒猫微服的协议支持

| 协议 | 是否需要挂载 | 公网可达 | 适用 |
|------|-------------|---------|------|
| **WebDAV（HTTPS）** | ❌ 否，纯 HTTP | ✅ `*.heiyu.space` | ⭐ 首选 |
| **hclient-cli 组网** | ❌ 否，组网隧道 | ✅ `*.heiyu.space` | 访问内网 Web 服务（gitblit 等） |
| SMB | ✅ 是 | 部分 | ❌ 不符合要求 |
| NFS | ✅ 是 | 否 | ❌ 不符合要求 |

## WebDAV 接入信息

- **协议**：WebDAV over HTTPS
- **地址**：懒猫网盘「网络服务 → WebDAV」界面提供的域名（`*.heiyu.space`）
- **认证**：该界面提供的**用户名 + 密码**（Basic Auth）
- **端口**：443（懒猫转发无限制，公网可直接访问）
- **路径**：填完地址会自动带出，支持子目录

## WaLiOffice 集成设计建议

### 1. 配置（按用户，存 user_settings）

```json
{
  "nas_webdav": {
    "base_url": "https://<微服名>.heiyu.space/dav/...",
    "username": "<webdav用户名>",
    "password": "<webdav密码>"
  }
}
```

> ⚠️ 密码需加密存储（现有 feishu_token 的加密逻辑可复用）。

### 2. 后端工具（纯 reqwest HTTP，无挂载）

| 工具 | HTTP 方法 | 用途 |
|------|----------|------|
| `nas_list` | PROPFIND（Depth:1） | 列目录/文件 |
| `nas_read` | GET | 读文件内容 |
| `nas_write` | PUT | 上传/写文件 |
| `nas_mkdir` | MKCOL | 建目录 |
| `nas_delete` | DELETE | 删除 |

### 3. 依赖

- Rust `reqwest`（项目已依赖）+ `dav-server` 或手写 PROPFIND XML 解析
- PROPFIND 返回 XML（`multistatus`），需解析 `d:href` + `d:propstat` + `d:displayname`

## 参考来源

- 懒猫网盘应用商店页：`https://lazycat.cloud/appstore/detail/cloud.lazycat.shell.files`
- Obsidian 同步实践（WebDAV）：懒猫微服「网络服务 → webdav」给域名/用户名/密码
- 官方 hclient-cli 外网访问教程：`https://lazycat.cloud/playground/guideline/1428`

## 待确认（已全部解决）

1. ~~达林提供实际的 WebDAV 域名 + 用户名 + 密码~~ → 已提供并接入。
2. ~~只读还是读写~~ → 读写（多数据源 + 来源/目的解构）。
3. ~~数据源形态：文件来源还是独立工具~~ → 独立工具（`nas_list` / `nas_read` / `nas_write` / `nas_mkdir`），同时作为生图/生视频的 `nas_refs` 引用源。

## 与本项目其他集成的关系

- 已有 `lazycat-nas-network` skill 讲的是 **hclient-cli 组网**（访问 gitblit），不是文件访问。
- 本次调研补充的是 **WebDAV 文件访问**（不挂载），两者互补：组网用于 git/内网服务，WebDAV 用于文件数据源。
