# roswire Website Assets

这个目录存放 `docs/index.html` 项目主页的静态资源，其中包含交互式 3D 工作原理演示。

3D 模块用于直观展示 `roswire` 的核心执行链路：

- Agent/脚本如何触发 roswire CLI
- roswire 如何进行 schema 自描述发现与 profile/secret 合并
- 协议探测与路由（REST / API / SSH）
- 默认只读、`--dry-run`、显式写入开关等安全边界
- `stdout` 成功 JSON 与 `stderr` 结构化错误的回流路径

## 文件说明

- `../index.html`：网站主页入口（项目介绍、快速开始、文档导航、3D 演示）。
- `styles.css`：主页整体视觉系统与响应式布局。
- `app.js`：Three.js 场景、阶段切换、图例渲染、数据流动画逻辑。

## 使用方式

直接在浏览器打开 `docs/index.html` 即可（需要联网加载 Three.js CDN）。

如果你希望以本地静态服务方式预览，可在仓库根目录启动静态服务并访问 `docs/`。

## 设计边界

- 这是项目官网前端，不是 RouterOS 实际连接器。
- 页面中的命令是示例语义，不会在浏览器中执行真实 `roswire` 命令。
- 演示强调安全约束：默认只读、secret 脱敏、流隔离与可观测诊断。
