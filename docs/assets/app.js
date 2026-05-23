import * as THREE from "three";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";

const STAGES = [
  {
    id: "Stage 1",
    short: "Agent request",
    title: "Agent 发起 JSON-first 请求",
    description:
      "自动化脚本或 AI Agent 触发 roswire 命令入口，目标是稳定机器可读输出，而不是交互式终端体验。",
    command: "roswire --json commands",
    notes: [
      "默认走结构化输出，减少 Agent 解析歧义",
      "成功结果写 stdout，诊断与错误写 stderr",
      "不展示颜色、spinner、分页器等人类终端特性"
    ],
    focusNodes: ["agent", "cli"],
    activeEdges: ["agent-cli"],
    packetEdges: ["agent-cli"],
    packetColor: 0x22eed6,
    cameraPos: new THREE.Vector3(-11, 6, 16),
    lookAt: new THREE.Vector3(-8.2, 2.2, 0)
  },
  {
    id: "Stage 2",
    short: "Schema discovery",
    title: "Schema / Command 自描述发现",
    description:
      "roswire 先读取命令目录与 schema，确保 Agent 知道参数契约与可执行能力，降低误调用概率。",
    command: "roswire --json schema command ip route print",
    notes: [
      "先 discover，再执行，避免盲目猜参数",
      "可结合 --remote 叠加真实设备能力",
      "错误码与修复提示保持稳定，可做自愈循环"
    ],
    focusNodes: ["cli", "schema", "router"],
    activeEdges: ["cli-schema", "schema-router", "cli-router"],
    packetEdges: ["cli-schema", "schema-router"],
    packetColor: 0x9f6bff,
    cameraPos: new THREE.Vector3(-6, 8, 16),
    lookAt: new THREE.Vector3(-3.2, 2.4, 0)
  },
  {
    id: "Stage 3",
    short: "Profile + secrets",
    title: "Profile 与 Secret 后端融合",
    description:
      "命令行参数、profile 配置、secret 引用在上下文层合并，密码等敏感信息只保留引用并始终脱敏。",
    command: "roswire --json --profile studio config inspect",
    notes: [
      "secret value 不应出现在日志/输出明文",
      "优先 keychain / env 引用而非硬编码密码",
      "支持来源追踪，便于 Agent 解释最终配置生效路径"
    ],
    focusNodes: ["cli", "profile", "router"],
    activeEdges: ["cli-profile", "profile-router", "cli-router"],
    packetEdges: ["cli-profile", "profile-router"],
    packetColor: 0xffc77a,
    cameraPos: new THREE.Vector3(-5, 6.6, 14),
    lookAt: new THREE.Vector3(-2.5, 1.2, 0)
  },
  {
    id: "Stage 4",
    short: "Protocol probing",
    title: "协议探测与自动路由",
    description:
      "在 auto 模式下，roswire 根据设备能力优先探测 REST，再回退 api-ssl/api，并路由到可用端点。",
    command: "roswire --json --profile studio doctor --include-remote",
    notes: [
      "network unreachable 会继续尝试候选协议",
      "authentication failure 直接终止，避免掩盖凭据问题",
      "RouterOS v6/v7 方言差异由内部层处理"
    ],
    focusNodes: ["router", "rest", "api", "ssh", "device"],
    activeEdges: [
      "router-rest",
      "router-api",
      "router-ssh",
      "rest-device",
      "api-device",
      "ssh-device"
    ],
    packetEdges: [
      "router-rest",
      "rest-device",
      "router-api",
      "api-device",
      "router-ssh",
      "ssh-device"
    ],
    packetColor: 0x6bf0cf,
    cameraPos: new THREE.Vector3(4.8, 8.5, 18),
    lookAt: new THREE.Vector3(5, 2.6, -1)
  },
  {
    id: "Stage 5",
    short: "Read-only / dry-run",
    title: "只读命令与 dry-run 安全执行",
    description:
      "执行只读 print 与传输预演时，数据流会经过安全门控；只有显式允许才进入写入动作。",
    command:
      "roswire --json --profile studio file upload ./setup.rsc flash/setup.rsc --dry-run",
    notes: [
      "raw 默认只读，建议仅允许 /.../print",
      "写操作需显式 --allow-write（本演示不触发）",
      "优先 dry-run 评估风险，再决定是否变更"
    ],
    focusNodes: ["cli", "router", "ssh", "device", "stdout"],
    activeEdges: ["cli-router", "router-ssh", "ssh-device", "device-stdout", "stdout-cli"],
    packetEdges: ["cli-router", "router-ssh", "ssh-device", "device-stdout"],
    packetColor: 0x86fff6,
    cameraPos: new THREE.Vector3(1.8, 5.5, 14),
    lookAt: new THREE.Vector3(4, 1.7, -2)
  },
  {
    id: "Stage 6",
    short: "Structured JSON result",
    title: "结构化结果回流（stdout / stderr）",
    description:
      "成功路径回流标准 JSON 到 stdout，失败路径返回结构化错误到 stderr，便于 Agent 稳定解析与总结。",
    command: "roswire --json --profile studio ip route print",
    notes: [
      "stdout 仅放成功结果，减少解析歧义",
      "stderr 提供错误码、脱敏上下文和诊断信息",
      "Agent 可据此做自动重试或修复建议"
    ],
    focusNodes: ["device", "stdout", "stderr", "cli"],
    activeEdges: ["device-stdout", "stdout-cli", "device-stderr", "stderr-cli"],
    packetEdges: ["device-stdout", "stdout-cli", "device-stderr", "stderr-cli"],
    packetColor: 0xbca8ff,
    cameraPos: new THREE.Vector3(9.8, 5.7, 16),
    lookAt: new THREE.Vector3(9.2, 2.1, 0)
  }
];

const NODE_DEFINITIONS = [
  {
    id: "agent",
    label: "Agent / LLM",
    color: 0xd5f4ff,
    accent: 0x22eed6,
    coreColor: 0x7ffff2,
    position: [-13, 3, 0],
    shape: "sphere",
    metalness: 0.12,
    roughness: 0.1,
    baseEmissive: 0.2,
    auraOpacity: 0.19,
    auraScale: 1.26
  },
  {
    id: "cli",
    label: "roswire CLI",
    color: 0x313844,
    accent: 0xffa24b,
    position: [-8.5, 1.6, 0],
    shape: "box",
    metalness: 0.92,
    roughness: 0.32,
    baseEmissive: 0.17,
    auraOpacity: 0.11,
    auraScale: 1.11
  },
  {
    id: "schema",
    label: "Schema Engine",
    color: 0xd8ccff,
    accent: 0x9f6bff,
    position: [-4.8, 4.6, 1.2],
    shape: "octa",
    metalness: 0,
    roughness: 0.05,
    baseEmissive: 0.22,
    auraOpacity: 0.16,
    auraScale: 1.21
  },
  {
    id: "profile",
    label: "Profile + Secret",
    color: 0xffa188,
    accent: 0xffcd7a,
    position: [-4.8, -1, -1.2],
    shape: "capsule",
    metalness: 0.33,
    roughness: 0.24,
    baseEmissive: 0.16,
    auraOpacity: 0.12,
    auraScale: 1.15
  },
  {
    id: "router",
    label: "Protocol Router",
    color: 0xff88e4,
    accent: 0xbd7bff,
    position: [-0.6, 1.8, 0],
    shape: "box",
    metalness: 0.42,
    roughness: 0.22,
    baseEmissive: 0.21,
    auraOpacity: 0.15,
    auraScale: 1.18
  },
  {
    id: "rest",
    label: "REST",
    color: 0x59f5b9,
    accent: 0x9fffe0,
    position: [4.5, 4.2, 2.5],
    shape: "sphere",
    metalness: 0.18,
    roughness: 0.28,
    baseEmissive: 0.18,
    auraOpacity: 0.13,
    auraScale: 1.14
  },
  {
    id: "api",
    label: "API / API-SSL",
    color: 0x7dcfff,
    accent: 0x5297ff,
    position: [4.8, 2, -2.8],
    shape: "sphere",
    metalness: 0.2,
    roughness: 0.3,
    baseEmissive: 0.18,
    auraOpacity: 0.12,
    auraScale: 1.14
  },
  {
    id: "ssh",
    label: "SSH Transfer",
    color: 0x44e4d2,
    accent: 0x86fff6,
    position: [4.3, -0.5, -4.8],
    shape: "sphere",
    metalness: 0.3,
    roughness: 0.22,
    baseEmissive: 0.17,
    auraOpacity: 0.12,
    auraScale: 1.13
  },
  {
    id: "device",
    label: "RouterOS Device",
    color: 0xf5d66d,
    accent: 0xffae61,
    position: [8.8, 1.9, -1.8],
    shape: "box",
    metalness: 0.58,
    roughness: 0.14,
    baseEmissive: 0.16,
    auraOpacity: 0.11,
    auraScale: 1.12
  },
  {
    id: "stdout",
    label: "stdout JSON",
    color: 0x99ffe5,
    accent: 0x4df8c1,
    position: [12.5, 3.4, 1.2],
    shape: "octa",
    metalness: 0.22,
    roughness: 0.3,
    baseEmissive: 0.2,
    auraOpacity: 0.15,
    auraScale: 1.18
  },
  {
    id: "stderr",
    label: "stderr Error",
    color: 0xff647a,
    accent: 0xffabb7,
    position: [12, 0.2, -2.4],
    shape: "octa",
    metalness: 0.2,
    roughness: 0.28,
    baseEmissive: 0.2,
    auraOpacity: 0.16,
    auraScale: 1.17
  }
];

const EDGE_DEFINITIONS = [
  { key: "agent-cli", from: "agent", to: "cli", lift: 1.2 },
  { key: "cli-schema", from: "cli", to: "schema", lift: 1.6 },
  { key: "cli-profile", from: "cli", to: "profile", lift: 1.1 },
  { key: "cli-router", from: "cli", to: "router", lift: 0.9 },
  { key: "schema-router", from: "schema", to: "router", lift: 1.2 },
  { key: "profile-router", from: "profile", to: "router", lift: 1.3 },
  { key: "router-rest", from: "router", to: "rest", lift: 1.6 },
  { key: "router-api", from: "router", to: "api", lift: 1.4 },
  { key: "router-ssh", from: "router", to: "ssh", lift: 1.2 },
  { key: "rest-device", from: "rest", to: "device", lift: 1.4 },
  { key: "api-device", from: "api", to: "device", lift: 1.2 },
  { key: "ssh-device", from: "ssh", to: "device", lift: 1.0 },
  { key: "device-stdout", from: "device", to: "stdout", lift: 1.2 },
  { key: "stdout-cli", from: "stdout", to: "cli", lift: 2.4 },
  { key: "device-stderr", from: "device", to: "stderr", lift: 0.9 },
  { key: "stderr-cli", from: "stderr", to: "cli", lift: 1.8 }
];

const canvas = document.getElementById("demo-canvas");
const stageNav = document.getElementById("stage-nav");
const hudStageId = document.getElementById("hud-stage-id");
const hudStageTitle = document.getElementById("hud-stage-title");
const panelTitle = document.getElementById("panel-title");
const panelDescription = document.getElementById("panel-description");
const panelCommand = document.getElementById("panel-command");
const panelNotes = document.getElementById("panel-notes");
const legendList = document.getElementById("legend-list");

if (!canvas) {
  throw new Error("缺少 #demo-canvas，无法初始化 Three.js 场景");
}

if (!hasWebGL()) {
  renderStaticFallback();
} else {

const renderer = new THREE.WebGLRenderer({
  canvas,
  antialias: true,
  alpha: true,
  powerPreference: "high-performance"
});
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
renderer.outputColorSpace = THREE.SRGBColorSpace;

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x090f1d);
scene.fog = new THREE.Fog(0x090f1d, 22, 44);

const camera = new THREE.PerspectiveCamera(48, 1, 0.1, 130);
camera.position.set(-10, 6, 16);

const ambient = new THREE.HemisphereLight(0x86a8ff, 0x050810, 0.75);
scene.add(ambient);

const keyLight = new THREE.DirectionalLight(0x7deaff, 1.1);
keyLight.position.set(-7, 9, 9);
scene.add(keyLight);

const rimLight = new THREE.PointLight(0xffbe6d, 1.4, 50, 1.2);
rimLight.position.set(11, 4, 6);
scene.add(rimLight);

const grid = new THREE.GridHelper(56, 56, 0x33517b, 0x1b2842);
grid.position.y = -2.3;
grid.material.opacity = 0.28;
grid.material.transparent = true;
scene.add(grid);

const stars = createStars();
scene.add(stars);

const composer = new EffectComposer(renderer);
composer.addPass(new RenderPass(scene, camera));
const bloomPass = new UnrealBloomPass(new THREE.Vector2(1, 1), 0.72, 0.48, 0.2);
composer.addPass(bloomPass);

const nodes = new Map();
const edges = new Map();
const packetGroup = new THREE.Group();
scene.add(packetGroup);

for (const nodeDef of NODE_DEFINITIONS) {
  const node = createNode(nodeDef);
  scene.add(node.group);
  nodes.set(nodeDef.id, node);
}

for (const edgeDef of EDGE_DEFINITIONS) {
  const fromNode = nodes.get(edgeDef.from);
  const toNode = nodes.get(edgeDef.to);
  if (!fromNode || !toNode) {
    continue;
  }

  const curve = makeCurve(fromNode.group.position, toNode.group.position, edgeDef.lift);
  const tube = new THREE.Mesh(
    new THREE.TubeGeometry(curve, 48, 0.052, 10, false),
    new THREE.MeshStandardMaterial({
      color: 0x2c3f63,
      emissive: 0x1a2b47,
      emissiveIntensity: 0.22,
      transparent: true,
      opacity: 0.28,
      metalness: 0.2,
      roughness: 0.4
    })
  );
  scene.add(tube);

  edges.set(edgeDef.key, {
    ...edgeDef,
    curve,
    mesh: tube,
    activeMix: 0
  });
}

const packetMaterials = {
  main: new THREE.MeshBasicMaterial({ color: 0x57f0ff }),
  ghost: new THREE.MeshBasicMaterial({ color: 0x57f0ff, transparent: true, opacity: 0.35 })
};

const packets = [];
for (let i = 0; i < 18; i += 1) {
  const mat = i % 3 === 0 ? packetMaterials.ghost.clone() : packetMaterials.main.clone();
  const mesh = new THREE.Mesh(new THREE.SphereGeometry(0.09, 12, 12), mat);
  mesh.visible = false;
  packetGroup.add(mesh);
  packets.push({
    mesh,
    edgeKey: null,
    t: Math.random(),
    speed: 0.14 + Math.random() * 0.24,
    reverse: i % 2 === 0
  });
}

const state = {
  stageIndex: 0,
  activeNodeSet: new Set(),
  activeEdgeSet: new Set(),
  targetCamPos: STAGES[0].cameraPos.clone(),
  targetLookAt: STAGES[0].lookAt.clone(),
  lookAt: STAGES[0].lookAt.clone()
};

const prefersReducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
let autoplayTimer = null;

const navButtons = STAGES.map((stage, index) => {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "stage-btn";
  button.role = "tab";
  button.textContent = `${index + 1}. ${stage.short}`;
  button.addEventListener("click", () => {
    applyStage(index, true);
  });
  stageNav.appendChild(button);
  return button;
});

renderLegend();

applyStage(0, false);
resize();
window.addEventListener("resize", resize);

const clock = new THREE.Clock();
renderer.setAnimationLoop(() => {
  const dt = clock.getDelta();
  const time = clock.elapsedTime;

  stars.rotation.y += dt * 0.01;

  for (const node of nodes.values()) {
    const target = state.activeNodeSet.has(node.id) ? 1 : 0;
    node.activeMix = THREE.MathUtils.lerp(node.activeMix, target, 0.09);

    const wave = 0.06 * Math.sin(time * 2.2 + node.phase);
    const glowPulse = 0.04 * Math.sin(time * 2.8 + node.phase * 1.3);
    const emissive = node.baseEmissive + node.activeMix * 0.95 + wave;
    node.material.emissiveIntensity = Math.max(0.06, emissive);
    node.group.scale.setScalar(1 + node.activeMix * 0.09);
    node.ring.rotation.z += dt * (0.45 + node.activeMix * 0.95);
    node.ring.material.opacity = THREE.MathUtils.clamp(
      0.28 + node.activeMix * 0.52 + glowPulse,
      0.18,
      0.88
    );
    node.aura.scale.setScalar(node.auraBaseScale + node.activeMix * 0.16 + glowPulse);
    node.aura.material.opacity = THREE.MathUtils.clamp(
      node.auraBaseOpacity + node.activeMix * 0.18 + wave * 0.5,
      0.04,
      0.56
    );

    if (node.innerCore) {
      const corePulse = 0.14 * Math.sin(time * 3.3 + node.phase);
      node.innerCore.rotation.y += dt * (0.85 + node.activeMix * 1.45);
      node.innerCore.rotation.x += dt * 0.35;
      node.innerCore.scale.setScalar(0.9 + node.activeMix * 0.14 + corePulse);
      node.innerCore.material.emissiveIntensity = 0.8 + node.activeMix * 1.2 + corePulse * 1.2;
    }

    if (node.streamGroup) {
      node.streamGroup.rotation.y += dt * (0.75 + node.activeMix * 1.7);
      node.streamGroup.rotation.z += dt * 0.24;
      node.streamGroup.children.forEach((stream, streamIndex) => {
        stream.material.opacity = THREE.MathUtils.clamp(
          0.2 + node.activeMix * 0.38 + 0.12 * Math.sin(time * 2.7 + streamIndex + node.phase),
          0.08,
          0.9
        );
      });
    }

    if (node.matrixShell) {
      node.matrixShell.rotation.y -= dt * 0.22;
      node.matrixShell.material.opacity = THREE.MathUtils.clamp(
        0.07 + node.activeMix * 0.2 + wave * 0.4,
        0.03,
        0.35
      );
    }

    if (node.cliFrame) {
      node.cliFrame.rotation.y += dt * (0.12 + node.activeMix * 0.28);
      node.cliFrame.material.opacity = THREE.MathUtils.clamp(
        0.32 + node.activeMix * 0.28 + wave * 0.25,
        0.18,
        0.82
      );
    }

    if (node.cliFibers.length) {
      node.cliFibers.forEach((fiber, fiberIndex) => {
        fiber.material.opacity = THREE.MathUtils.clamp(
          0.2 + node.activeMix * 0.5 + 0.15 * Math.sin(time * 3.9 + fiberIndex * 1.6 + node.phase),
          0.1,
          0.95
        );
      });
    }

    if (node.prismBeam) {
      node.prismBeam.rotation.y += dt * (0.95 + node.activeMix * 1.5);
      node.prismBeam.material.opacity = THREE.MathUtils.clamp(
        0.2 + node.activeMix * 0.45 + glowPulse * 0.6,
        0.08,
        0.9
      );
    }

    if (node.prismRing) {
      node.prismRing.rotation.x += dt * (0.5 + node.activeMix * 0.9);
      node.prismRing.rotation.y += dt * 0.22;
      node.prismRing.material.opacity = THREE.MathUtils.clamp(
        0.25 + node.activeMix * 0.35 + glowPulse * 0.4,
        0.1,
        0.9
      );
    }

    if (node.prismEdges) {
      node.prismEdges.rotation.y += dt * (0.3 + node.activeMix * 0.7);
      node.prismEdges.material.opacity = THREE.MathUtils.clamp(
        0.25 + node.activeMix * 0.45 + wave * 0.6,
        0.08,
        0.95
      );
    }
  }

  for (const edge of edges.values()) {
    const target = state.activeEdgeSet.has(edge.key) ? 1 : 0;
    edge.activeMix = THREE.MathUtils.lerp(edge.activeMix, target, 0.1);

    const activeColor = new THREE.Color(0x66f5d7);
    const idleColor = new THREE.Color(0x2c3f63);
    edge.mesh.material.color.copy(idleColor).lerp(activeColor, edge.activeMix);
    edge.mesh.material.emissiveIntensity = 0.22 + edge.activeMix * 1.1;
    edge.mesh.material.opacity = 0.25 + edge.activeMix * 0.53;
  }

  for (const packet of packets) {
    if (!packet.edgeKey) {
      packet.mesh.visible = false;
      continue;
    }

    const edge = edges.get(packet.edgeKey);
    if (!edge) {
      packet.mesh.visible = false;
      continue;
    }

    packet.t += dt * packet.speed;
    if (packet.t > 1) {
      packet.t -= 1;
    }

    const sample = packet.reverse ? 1 - packet.t : packet.t;
    edge.curve.getPointAt(sample, packet.mesh.position);
    packet.mesh.visible = true;
  }

  camera.position.lerp(state.targetCamPos, 0.05);
  state.lookAt.lerp(state.targetLookAt, 0.07);
  camera.lookAt(state.lookAt);

  const bloomPulse = 0.04 * Math.sin(time * 0.72 + state.stageIndex * 0.8);
  bloomPass.strength = THREE.MathUtils.clamp(
    0.68 + bloomPulse + state.activeNodeSet.size * 0.008,
    0.56,
    0.92
  );

  composer.render();
});

function applyStage(index, fromUser) {
  state.stageIndex = index;
  const stage = STAGES[index];

  state.activeNodeSet = new Set(stage.focusNodes);
  state.activeEdgeSet = new Set(stage.activeEdges);
  state.targetCamPos.copy(stage.cameraPos);
  state.targetLookAt.copy(stage.lookAt);

  hudStageId.textContent = stage.id;
  hudStageTitle.textContent = stage.title;
  panelTitle.textContent = stage.title;
  panelDescription.textContent = stage.description;
  panelCommand.textContent = stage.command;

  panelNotes.innerHTML = "";
  stage.notes.forEach((note) => {
    const li = document.createElement("li");
    li.textContent = note;
    panelNotes.appendChild(li);
  });

  navButtons.forEach((btn, buttonIndex) => {
    btn.classList.toggle("active", buttonIndex === index);
    btn.setAttribute("aria-selected", buttonIndex === index ? "true" : "false");
  });

  assignPackets(stage.packetEdges, stage.packetColor);

  if (!prefersReducedMotion && fromUser) {
    scheduleAutoplay();
  }
}

function assignPackets(edgeKeys, colorHex) {
  if (!edgeKeys.length) {
    packets.forEach((packet) => {
      packet.edgeKey = null;
      packet.mesh.visible = false;
    });
    return;
  }

  packets.forEach((packet, index) => {
    const edgeKey = edgeKeys[index % edgeKeys.length];
    packet.edgeKey = edgeKey;
    packet.t = Math.random();
    packet.speed = 0.12 + Math.random() * 0.23;
    packet.mesh.material.color.setHex(colorHex);
    if (packet.mesh.material.opacity !== undefined) {
      packet.mesh.material.opacity = packet.mesh.material.opacity > 0.6 ? 1 : 0.35;
    }
  });
}

function scheduleAutoplay() {
  if (autoplayTimer) {
    window.clearInterval(autoplayTimer);
  }

  autoplayTimer = window.setInterval(() => {
    const next = (state.stageIndex + 1) % STAGES.length;
    applyStage(next, false);
  }, 6200);
}

if (!prefersReducedMotion) {
  scheduleAutoplay();
}

function resize() {
  const rect = canvas.getBoundingClientRect();
  const width = Math.max(320, rect.width);
  const height = Math.max(320, rect.height);

  renderer.setSize(width, height, false);
  composer.setSize(width, height);
  bloomPass.setSize(width, height);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}

function renderLegend() {
  if (!legendList) {
    return;
  }

  legendList.innerHTML = "";

  for (const def of NODE_DEFINITIONS) {
    const item = document.createElement("li");

    const swatch = document.createElement("span");
    swatch.className = "swatch";

    const swatchColor = new THREE.Color(def.accent ?? def.color);
    const [r, g, b] = swatchColor.toArray().map((channel) => Math.round(channel * 255));
    swatch.style.background = `#${swatchColor.getHexString()}`;
    swatch.style.boxShadow = `0 0 14px rgba(${r}, ${g}, ${b}, 0.72)`;

    const text = document.createElement("span");
    text.textContent = def.label;

    item.append(swatch, text);
    legendList.appendChild(item);
  }
}

function createNode(def) {
  const group = new THREE.Group();
  group.position.set(def.position[0], def.position[1], def.position[2]);
  const mainColor = new THREE.Color(def.color);
  const accentColor = new THREE.Color(def.accent ?? def.color);

  const geometry = pickGeometry(def.shape);
  const material = createNodeMaterial(def, mainColor, accentColor);

  const mesh = new THREE.Mesh(geometry, material);
  mesh.scale.setScalar(0.96);
  group.add(mesh);

  const aura = new THREE.Mesh(
    new THREE.SphereGeometry(0.96, 22, 18),
    new THREE.MeshBasicMaterial({
      color: accentColor,
      transparent: true,
      opacity: def.auraOpacity ?? 0.12,
      blending: THREE.AdditiveBlending,
      depthWrite: false
    })
  );
  aura.scale.setScalar(def.auraScale ?? 1.16);
  group.add(aura);

  const ring = new THREE.Mesh(
    new THREE.TorusGeometry(0.78, 0.04, 12, 64),
    new THREE.MeshBasicMaterial({ color: accentColor, transparent: true, opacity: 0.45 })
  );
  ring.rotation.x = Math.PI / 2;
  group.add(ring);

  const sprite = makeLabel(def.label, def.accent ?? def.color);
  sprite.position.set(0, 1.34, 0);
  group.add(sprite);

  const node = {
    id: def.id,
    group,
    material,
    aura,
    ring,
    innerCore: null,
    streamGroup: null,
    matrixShell: null,
    cliFrame: null,
    cliFibers: [],
    prismBeam: null,
    prismRing: null,
    prismEdges: null,
    activeMix: 0,
    baseEmissive: def.baseEmissive ?? 0.18,
    auraBaseScale: def.auraScale ?? 1.16,
    auraBaseOpacity: def.auraOpacity ?? 0.12,
    phase: Math.random() * Math.PI * 2
  };

  if (def.id === "agent") {
    const innerCore = new THREE.Mesh(
      new THREE.IcosahedronGeometry(0.28, 2),
      new THREE.MeshPhysicalMaterial({
        color: def.coreColor ?? def.accent ?? def.color,
        emissive: def.coreColor ?? def.accent ?? def.color,
        emissiveIntensity: 1,
        roughness: 0.08,
        metalness: 0.08,
        transmission: 0.22,
        thickness: 0.35,
        clearcoat: 1,
        clearcoatRoughness: 0.1
      })
    );
    group.add(innerCore);

    const streamGroup = new THREE.Group();
    for (let i = 0; i < 3; i += 1) {
      const stream = new THREE.Mesh(
        new THREE.TorusGeometry(0.42 + i * 0.05, 0.014, 10, 80),
        new THREE.MeshBasicMaterial({
          color: accentColor,
          transparent: true,
          opacity: 0.34,
          blending: THREE.AdditiveBlending,
          depthWrite: false
        })
      );
      stream.rotation.set(Math.PI / (2.2 + i * 0.3), i * 0.6, i * 0.4);
      streamGroup.add(stream);
    }
    group.add(streamGroup);

    const matrixShell = new THREE.Mesh(
      new THREE.SphereGeometry(0.74, 24, 20),
      new THREE.MeshBasicMaterial({
        color: accentColor,
        wireframe: true,
        transparent: true,
        opacity: 0.1,
        depthWrite: false
      })
    );
    group.add(matrixShell);

    node.innerCore = innerCore;
    node.streamGroup = streamGroup;
    node.matrixShell = matrixShell;
  }

  if (def.id === "cli") {
    const cliFrame = new THREE.LineSegments(
      new THREE.EdgesGeometry(new THREE.BoxGeometry(1.08, 1.08, 1.08)),
      new THREE.LineBasicMaterial({
        color: accentColor,
        transparent: true,
        opacity: 0.38
      })
    );
    group.add(cliFrame);

    const fiberGeo = new THREE.BoxGeometry(0.78, 0.03, 0.03);
    const fiberOffsets = [
      [0, 0.35, 0.54],
      [0, -0.35, 0.54],
      [0, 0.35, -0.54],
      [0, -0.35, -0.54]
    ];
    const cliFibers = fiberOffsets.map((offset) => {
      const fiber = new THREE.Mesh(
        fiberGeo,
        new THREE.MeshBasicMaterial({
          color: accentColor,
          transparent: true,
          opacity: 0.32,
          blending: THREE.AdditiveBlending,
          depthWrite: false
        })
      );
      fiber.position.set(offset[0], offset[1], offset[2]);
      group.add(fiber);
      return fiber;
    });

    node.cliFrame = cliFrame;
    node.cliFibers = cliFibers;
  }

  if (def.id === "schema") {
    const prismBeam = new THREE.Mesh(
      new THREE.CylinderGeometry(0.06, 0.06, 1.36, 18, 1, true),
      new THREE.MeshBasicMaterial({
        color: accentColor,
        transparent: true,
        opacity: 0.28,
        blending: THREE.AdditiveBlending,
        depthWrite: false
      })
    );
    group.add(prismBeam);

    const prismRing = new THREE.Mesh(
      new THREE.TorusGeometry(0.58, 0.018, 12, 82),
      new THREE.MeshBasicMaterial({
        color: accentColor,
        transparent: true,
        opacity: 0.32,
        blending: THREE.AdditiveBlending,
        depthWrite: false
      })
    );
    prismRing.rotation.x = Math.PI / 2.8;
    group.add(prismRing);

    const prismEdges = new THREE.LineSegments(
      new THREE.EdgesGeometry(new THREE.OctahedronGeometry(0.74, 0)),
      new THREE.LineBasicMaterial({
        color: accentColor,
        transparent: true,
        opacity: 0.35
      })
    );
    group.add(prismEdges);

    node.prismBeam = prismBeam;
    node.prismRing = prismRing;
    node.prismEdges = prismEdges;
  }

  return node;
}

function createNodeMaterial(def, mainColor, accentColor) {
  if (def.id === "agent") {
    return new THREE.MeshPhysicalMaterial({
      color: mainColor,
      emissive: accentColor,
      emissiveIntensity: 0.42,
      roughness: 0.1,
      metalness: 0.08,
      transmission: 0.72,
      ior: 1.5,
      thickness: 1,
      clearcoat: 1,
      clearcoatRoughness: 0.16,
      attenuationColor: accentColor,
      attenuationDistance: 1.8
    });
  }

  if (def.id === "cli") {
    return new THREE.MeshPhysicalMaterial({
      color: mainColor,
      emissive: accentColor,
      emissiveIntensity: 0.2,
      roughness: 0.34,
      metalness: 0.95,
      clearcoat: 1,
      clearcoatRoughness: 0.12
    });
  }

  if (def.id === "schema") {
    return new THREE.MeshPhysicalMaterial({
      color: mainColor,
      emissive: accentColor,
      emissiveIntensity: 0.52,
      roughness: 0.04,
      metalness: 0,
      transmission: 0.9,
      ior: 2.2,
      thickness: 1.1,
      clearcoat: 1,
      clearcoatRoughness: 0.03,
      iridescence: 0.9,
      iridescenceIOR: 1.4,
      iridescenceThicknessRange: [120, 520]
    });
  }

  return new THREE.MeshStandardMaterial({
    color: mainColor,
    emissive: accentColor,
    emissiveIntensity: def.emissiveIntensity ?? 0.24,
    metalness: def.metalness ?? 0.24,
    roughness: def.roughness ?? 0.35
  });
}

function pickGeometry(shape) {
  if (shape === "box") {
    return new THREE.BoxGeometry(1.05, 1.05, 1.05);
  }

  if (shape === "octa") {
    return new THREE.OctahedronGeometry(0.72, 0);
  }

  if (shape === "capsule") {
    return new THREE.CapsuleGeometry(0.44, 0.56, 6, 12);
  }

  return new THREE.SphereGeometry(0.66, 20, 18);
}

function makeCurve(start, end, lift = 1) {
  const mid = new THREE.Vector3().addVectors(start, end).multiplyScalar(0.5);
  mid.y += lift;
  mid.z += (start.z - end.z) * 0.1;
  return new THREE.QuadraticBezierCurve3(start.clone(), mid, end.clone());
}

function createStars() {
  const geometry = new THREE.BufferGeometry();
  const count = 1200;
  const positions = new Float32Array(count * 3);

  for (let i = 0; i < count; i += 1) {
    const i3 = i * 3;
    positions[i3] = (Math.random() - 0.5) * 80;
    positions[i3 + 1] = Math.random() * 32 - 8;
    positions[i3 + 2] = (Math.random() - 0.5) * 80;
  }

  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));

  const material = new THREE.PointsMaterial({
    size: 0.08,
    color: 0x8fb0ff,
    transparent: true,
    opacity: 0.45,
    depthWrite: false
  });

  return new THREE.Points(geometry, material);
}

function makeLabel(text, color) {
  const canvas2d = document.createElement("canvas");
  const ctx = canvas2d.getContext("2d");

  if (!ctx) {
    throw new Error("无法创建 2D 画布上下文用于标签渲染");
  }

  const width = 420;
  const height = 126;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  canvas2d.width = Math.floor(width * dpr);
  canvas2d.height = Math.floor(height * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

  const colorHex = `#${new THREE.Color(color).getHexString()}`;

  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = "rgba(6, 10, 22, 0.9)";
  roundRect(ctx, 10, 24, width - 20, 72, 14);
  ctx.fill();

  ctx.strokeStyle = `${colorHex}D9`;
  ctx.lineWidth = 2.4;
  roundRect(ctx, 10, 24, width - 20, 72, 14);
  ctx.stroke();

  ctx.fillStyle = "#f3f7ff";
  ctx.font = "700 30px IBM Plex Sans, Source Han Sans SC, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.shadowColor = "rgba(10, 18, 34, 0.95)";
  ctx.shadowBlur = 4;
  ctx.fillText(text, width / 2, 60);
  ctx.shadowBlur = 0;

  const texture = new THREE.CanvasTexture(canvas2d);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = Math.min(renderer.capabilities.getMaxAnisotropy(), 8);

  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({
      map: texture,
      transparent: true,
      depthWrite: false,
      depthTest: false
    })
  );

  sprite.scale.set(3.6, 1.08, 1);
  return sprite;
}

function roundRect(ctx, x, y, width, height, radius) {
  const r = Math.min(radius, width / 2, height / 2);
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.lineTo(x + width - r, y);
  ctx.quadraticCurveTo(x + width, y, x + width, y + r);
  ctx.lineTo(x + width, y + height - r);
  ctx.quadraticCurveTo(x + width, y + height, x + width - r, y + height);
  ctx.lineTo(x + r, y + height);
  ctx.quadraticCurveTo(x, y + height, x, y + height - r);
  ctx.lineTo(x, y + r);
  ctx.quadraticCurveTo(x, y, x + r, y);
  ctx.closePath();
}
}

function hasWebGL() {
  try {
    const testCanvas = document.createElement("canvas");
    return Boolean(testCanvas.getContext("webgl2") || testCanvas.getContext("webgl"));
  } catch {
    return false;
  }
}

function renderStaticFallback() {
  canvas.classList.add("is-hidden");
  canvas.setAttribute("aria-hidden", "true");

  const fallback = document.createElement("div");
  fallback.className = "canvas-fallback";
  fallback.setAttribute("role", "status");
  const fallbackTitle = document.createElement("strong");
  fallbackTitle.textContent = "3D 演示当前不可用";
  const fallbackMessage = document.createElement("span");
  fallbackMessage.textContent = "当前浏览器或运行环境未提供 WebGL，已切换为静态阶段说明。";
  fallback.append(fallbackTitle, fallbackMessage);
  canvas.insertAdjacentElement("afterend", fallback);

  const navButtons = STAGES.map((stage, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "stage-btn";
    button.setAttribute("role", "tab");
    button.textContent = `${index + 1}. ${stage.short}`;
    button.addEventListener("click", () => {
      applyStaticStage(index);
    });
    stageNav.appendChild(button);
    return button;
  });

  function applyStaticStage(index) {
    const stage = STAGES[index];

    hudStageId.textContent = stage.id;
    hudStageTitle.textContent = stage.title;
    panelTitle.textContent = stage.title;
    panelDescription.textContent = stage.description;
    panelCommand.textContent = stage.command;

    panelNotes.innerHTML = "";
    stage.notes.forEach((note) => {
      const li = document.createElement("li");
      li.textContent = note;
      panelNotes.appendChild(li);
    });

    navButtons.forEach((btn, buttonIndex) => {
      btn.classList.toggle("active", buttonIndex === index);
      btn.setAttribute("aria-selected", buttonIndex === index ? "true" : "false");
    });
  }

  renderStaticLegend();
  applyStaticStage(0);
}

function renderStaticLegend() {
  if (!legendList) {
    return;
  }

  legendList.innerHTML = "";

  for (const def of NODE_DEFINITIONS) {
    const item = document.createElement("li");

    const swatch = document.createElement("span");
    const color = `#${(def.accent ?? def.color).toString(16).padStart(6, "0")}`;
    swatch.className = "swatch";
    swatch.style.background = color;
    swatch.style.boxShadow = `0 0 14px ${color}B8`;

    const text = document.createElement("span");
    text.textContent = def.label;

    item.append(swatch, text);
    legendList.appendChild(item);
  }
}
