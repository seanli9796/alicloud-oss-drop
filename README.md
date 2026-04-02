# OSS Drop

本地桌面版阿里云 OSS 上传工具（Tauri + Node）。  
A local desktop uploader for Alibaba Cloud OSS (Tauri + Node).

## 功能概览 | Features

- 拖拽/选择文件并上传到 OSS 指定路径。  
  Drag or pick files and upload to a target OSS prefix.
- 首次填写 AK/SK 后本地保存，后续自动读取。  
  Save AK/SK locally on first run, then auto-load next time.
- 支持主界面快速切换 Bucket，并联动 Region。  
  Switch bucket directly on the main screen with region linkage.
- 支持路径列表选择 + 手动输入路径。  
  Supports both prefix list selection and manual path input.
- 支持 Cmd/Ctrl + K 快捷操作与最近路径书签。  
  Supports Cmd/Ctrl + K quick actions and recent path bookmarks.

## 快速开始 | Quick Start

### 1) 安装依赖 | Install Dependencies

```bash
npm install
```

### 2) 可选：本地 Node 调试环境变量 | Optional: Local Node Debug Env

```bash
cp .env.example .env
```

编辑 `.env`：  
Edit `.env`:

```dotenv
OSS_ACCESS_KEY_ID=yourAccessKeyID
OSS_ACCESS_KEY_SECRET=yourAccessKeySecret
OSS_BUCKET=yourBucketName
OSS_REGION=oss-cn-hongkong
OSS_PREFIX=uploads/
UPLOAD_PASSWORD=optionalPassword
```

### 3) 启动 Web 服务（Node）| Start Node Service

```bash
# 生产模式 | production
npm start

# 开发模式（自动重启）| dev mode with auto-restart
npm run dev
```

访问 / Open: [http://127.0.0.1:3001](http://127.0.0.1:3001)

### 4) 启动 Tauri 桌面版 | Run Tauri Desktop

```bash
npm run tauri:dev
```

首次配置建议先填 `Access Key ID` + `Access Key Secret`（Bucket 可后续在主界面选择）。  
On first setup, fill `Access Key ID` + `Access Key Secret` first (bucket can be selected later on the main screen).

配置会保存到本地，后续启动自动读取。  
Configuration is stored locally and auto-loaded on next launch.

### 5) 打包 macOS 安装包 | Build macOS Package

```bash
npm run tauri:build
```

## 常用脚本 | Useful Scripts

```bash
# 同步前端页面到 tauri-ui
npm run sync:tauri-ui

# 清理 Tauri 构建缓存（释放磁盘）
npm run clean:build-cache
```

## 部署（内网 Node 服务）| Deployment (Internal Node Service)

```bash
# 安装 PM2 | install PM2
npm install -g pm2

# 启动服务 | start service
pm2 start server.js --name oss-uploader

# 开机自启 | startup on boot
pm2 startup
pm2 save
```

## 安全建议 | Security Notes

1. 不要提交 `.env`（已在 `.gitignore`）。  
   Do not commit `.env` (already ignored).
2. 可配置 `UPLOAD_PASSWORD` 防止未授权访问。  
   Set `UPLOAD_PASSWORD` for basic access protection.
3. 建议在阿里云 RAM 给 AccessKey 配置最小权限与 IP 白名单。  
   Use least-privilege RAM policy and IP allowlist for AccessKey.
4. 不要使用主账号长期密钥，建议使用 RAM 子账号。  
   Avoid root account keys; use RAM sub-account credentials.
