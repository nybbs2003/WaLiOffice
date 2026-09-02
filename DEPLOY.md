# Moe Office 云服务器部署说明

> 当前生产部署（spark1.lab207.cn，阿里云 47.106.98.132）采用**单一 Rust 二进制**部署：
> 本地/CI 构建后 scp 二进制 + `.env` 到 `/opt/walioffice`，以 systemd 常驻。
> 下方 Docker 流程为可选的容器化部署路径（上游原流程，镜像仓库已换成本 fork 自持）。

## 完整流程

### 第一步：本地构建并推送镜像

```bash
# 1. 启动 Docker Desktop
# 2. 进入项目目录
cd ~/git/WaLiOffice-fork

# 3. 构建并推送（build.sh 需自持镜像仓库凭据）
./build.sh
```

构建完成后镜像地址（以实际自持仓库为准）：
```
registry.cn-hangzhou.aliyuncs.com/<your-namespace>/walioffice:1.0
```

### 第二步：云服务器部署

```bash
# 1. 安装 Docker（Ubuntu/Debian）
curl -fsSL https://get.docker.com | sh
sudo systemctl enable docker && sudo systemctl start docker

# 2. 创建部署目录
mkdir -p /opt/walioffice && cd /opt/walioffice

# 3. 创建 docker-compose-walioffice.yml（从仓库复制或手写）
# 4. 创建 .env 文件
cp .env.production .env  # 如果有模板
vim .env                 # 修改以下关键配置:
```

**必须修改的 .env 配置**：
| 配置项 | 修改为 |
|--------|--------|
| `AIPPT_JWT_SECRET` | `openssl rand -hex 32` 生成的随机字符串 |
| `LLM_TEXT_API_KEY` | 你的文本 LLM API Key |
| `LLM_IMAGE_API_KEY` | 你的图片模型 API Key |
| `LLM_VIDEO_API_KEY` | 你的视频模型 API Key |
| `AIPPT_CORS_ORIGINS` | 你的域名或 `http://服务器IP:8000` |

```bash
# 5. 登录阿里云镜像仓库
docker login --username="小傅哥" registry.cn-hangzhou.aliyuncs.com

# 6. 拉取镜像并启动
docker compose pull
docker compose up -d

# 7. 验证
docker compose logs -f
curl http://localhost:8000
```

### 第三步（可选）：Nginx + HTTPS

```bash
# 安装 Nginx
sudo apt install nginx -y

# 申请 SSL 证书
sudo apt install certbot python3-certbot-nginx -y
sudo certbot --nginx -d your-domain.com
```

Nginx 配置参考：
```nginx
server {
    listen 443 ssl http2;
    server_name your-domain.com;

    ssl_certificate     /etc/letsencrypt/live/your-domain.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/your-domain.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # SSE 流式响应支持
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 600s;
    }
}

server {
    listen 80;
    server_name your-domain.com;
    return 301 https://$host$request_uri;
}
```

## 常用运维命令

```bash
# 查看日志
docker compose logs -f

# 重启服务
docker compose restart

# 停止服务
docker compose down

# 更新镜像（重新构建推送后）
docker compose pull && docker compose up -d

# 备份数据
cp -r ./data ./data-backup-$(date +%Y%m%d)
```

## 文件说明

| 文件 | 用途 |
|------|------|
| `build.sh` | 本地构建多平台镜像并推送到阿里云 |
| `.local-config` | 阿里云镜像仓库登录凭证（不提交 Git） |
| `Dockerfile` | 三阶段构建（前端 → Rust 后端 → 运行时） |
| `.dockerignore` | Docker 构建排除文件 |
| `docker-compose.yml` | 云服务器部署配置（从镜像库拉取） |
| `.env.production` | 生产环境配置模板 |
| `.env` | 实际运行配置（不提交 Git） |
