# ColorYourModel 公开反馈服务

桌面端顶栏「反馈」和「帮助 → 反馈问题」打开应用内的反馈窗口，并预填语言和应用版本。用户选填系统和复现步骤，确认公开发布并通过人机验证后，GitHub App 会在 `ZackaryShen/ColorYourModel` 创建 Issue。成功页显示 Issue 编号，点击链接可在浏览器查看 GitHub 讨论。表单只接收文字。

## 部署

1. 在 GitHub 创建 GitHub App，关闭 Webhook，授予仓库权限 **Issues: Read and write**，只安装到 `ZackaryShen/ColorYourModel`。记录 **Client ID** 和 **Installation ID**，并生成该 App 的私钥。安装 ID 可在应用安装配置页的 URL 中找到。
2. 登录 Cloudflare Workers。为 Worker 的公开主机名创建 **Managed** Turnstile widget。默认地址是 `coloryourmodel-feedback.<你的子域>.workers.dev`。使用自定义域名时，在 Turnstile 注册实际打开表单的域名。
3. 在本目录外的受保护位置准备一个 JSON 密钥文件。四个值为 `GITHUB_APP_CLIENT_ID`、`GITHUB_INSTALLATION_ID`、`GITHUB_PRIVATE_KEY`、`TURNSTILE_SECRET`。它们在 JSON 中均为字符串。`GITHUB_PRIVATE_KEY` 使用未加密 PKCS#8 PEM，开头为 `-----BEGIN PRIVATE KEY-----`。GitHub 下载的 PEM 可用 `openssl pkcs8 -topk8 -nocrypt -in github-app.pem` 转换。保留 PEM 的换行，写入 JSON 时编码为 `\n`。
4. 从仓库根目录执行以下命令。`TURNSTILE_SITE_KEY` 是公开的 widget site key，`--secrets-file` 指向上一步的密钥文件。

   ```sh
   cd feedback-worker
   npx wrangler@4 login
   npx wrangler@4 deploy --secrets-file /path/to/feedback-secrets.json --var "TURNSTILE_SITE_KEY:<site-key>"
   ```

5. 打开部署输出的 HTTPS 地址，确认表单加载。在真实仓库发测试 Issue 前先约定测试标题和清理方式。提交后核对作者为 GitHub App、标签正确、链接可打开。
6. 在 GitHub 仓库的 **Settings → Secrets and variables → Actions → Variables** 设置 `VITE_FEEDBACK_URL` 为 Worker 的 HTTPS 根地址。发布工作流会在打包时读取这个变量。独立构建默认使用官方表单，也可传入该变量覆盖地址。显式传入空值会将反馈入口连接到 GitHub 自带的模板页。

```sh
VITE_FEEDBACK_URL=https://coloryourmodel-feedback.example.com npm run tauri build
```

## 本地验证

从仓库根目录运行以下命令。Worker 测试模拟 Turnstile 和 GitHub 响应，并检查实际签名和发帖请求。不会写入公开仓库。

```sh
node --test feedback-worker/worker.test.mjs
npx vitest run src/components/MenuBar.feedback.test.tsx
```
