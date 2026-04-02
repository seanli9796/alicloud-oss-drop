# OSS Drop

本地桌面版阿里云 OSS 上传工具。首次填写一套 AK / SK / Bucket / Region 后，本地保存，后续直接打开使用。

## 快速开始

### 1. 安装依赖

```bash
npm install
```

### 2. 配置环境变量（可选，仅本地 Node 调试接口时需要）

```bash
cp .env.example .env
```

编辑 `.env` 文件，填入你的配置：

```
OSS_ACCESS_KEY_ID=你的AccessKeyID
OSS_ACCESS_KEY_SECRET=你的AccessKeySecret
OSS_BUCKET=你的Bucket名称
OSS_REGION=oss-cn-hongkong
OSS_PREFIX=uploads/
UPLOAD_PASSWORD=可选的访问密码
```

### 3. 启动服务

```bash
# 生产环境
npm start

# 开发（文件变动自动重启，Node.js 18+）
npm run dev
```

打开 http://127.0.0.1:3001 即可使用。

### 4. 启动 Tauri 桌面版

```bash
# 开发模式启动 Tauri
npm run tauri:dev
```

Tauri 桌面版首次启动会要求填写：

1. `Access Key ID`
2. `Access Key Secret`
3. `Bucket`
4. `Region`
这些配置会保存在当前用户本地，后续再次打开会自动读取，不需要再输入密码或 Key。

后续使用里可以直接在主界面快速切换 `Bucket`，不需要重新填写整套配置。

Tauri 版会自动：

1. 启动本地上传服务
2. 自动寻找空闲端口
3. 打开内嵌窗口，不需要再手动开浏览器

如果要打包成 macOS 安装包：

```bash
npm run tauri:build
```

---

## 部署到公司内网服务器

```bash
# 安装 PM2（进程守护）
npm install -g pm2

# 启动
pm2 start server.js --name oss-uploader

# 开机自启
pm2 startup
pm2 save
```

## 安全建议

1. `.env` 文件不要提交到 Git（已加入 `.gitignore`）
2. 设置 `UPLOAD_PASSWORD` 防止未授权访问
3. 在阿里云 RAM 控制台为该 AccessKey 配置 IP 白名单（只允许服务器 IP）
4. 建议给 RAM 子账号只授权 `AliyunOSSFullAccess`，不要用主账号 key
