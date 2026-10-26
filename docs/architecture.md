# 架构

本文描述 RuStory 的分层、数据流和不可违背的不变量。
决策的**理由**在 [`adr/`](adr/) 里，这里只讲**现在的形状**。

---

## 1. 一句话结构

```
剧本 (.ron)  ──▶  Story  ──▶  Narrative  ──▶  Frame  ──▶  wgpu  ──▶  像素
                                │
                                └──▶  GameplaySystem（可插拔）
```

叙事是唯一的状态所有者。渲染器和玩法系统都不持有游戏状态。

---

## 2. 四个 crate 与依赖方向

```
        apps/rustory
        ├── rustory-render
        ├── rustory-script
        ├── rustory-gameplay-timer
        └── rustory-core
                ▲
   rustory-render ─┘        （只依赖 core 的类型，不被 core 依赖）
   rustory-script ─┘
   rustory-gameplay-timer ─┘
```

**依赖永远单向指向 `rustory-core`。** core 不依赖任何上层。
这条用 Cargo 的依赖声明保证，不靠自觉。

| crate | 职责 | 不该做的事 |
|---|---|---|
| `rustory-core` | 故事模型、状态机、玩法契约 | 渲染、窗口、文件 IO |
| `rustory-script` | RON 解析、结构校验 | 播放、状态推进 |
| `rustory-render` | 绘制列表 → 像素 | 认识「剧情」「角色」「选择」 |
| `rustory-gameplay-timer` | 一个具体玩法 | 读舞台、读故事图 |

---

## 3. 数据流

### 3.1 一次推进

```
玩家点击
  → app 翻译成意图（advance / choose / select）
  → Narrative 改变状态，产出 Vec<NarrativeEvent>
  → app 每帧重新构建 Frame（无增量镜像）
  → Renderer 把 Frame 画出来
```

渲染器**没有状态缓存**。每帧从 `Narrative` 重新读一遍。
这是有意的：少一个需要同步的镜像，就少一类「画面和状态不一致」的 bug。

### 3.2 玩法接管

```
剧本节点 exit: Gameplay(spec)
  → Narrative.mode = Suspended { spec }
  → app 每帧调 Narrative.step_gameplay(registry, dt, inputs)
      → 首次调用 begin(config)，玩法解析自己的配置
      → 之后每帧 step(ctx)，玩法返回 Option<OutcomeKey>
  → 玩法返回 Some(key)
      → Narrative 查 spec.outcomes[key] 得到下一个节点
      → Narrative.mode 回到播放
```

关键：**核心不解析 config，也不解释 key 的含义。**

---

## 4. 不变量

这些是「改了就会破坏架构承诺」的东西。改动前先写 ADR。

1. **`rustory-core` 不依赖任何渲染或窗口代码。**
   违反它，核心就不能在无 GPU 环境测试，测试成本会涨一个数量级。

2. **玩法系统拿不到舞台、故事图或渲染器。**
   它能拿到的全部是 `GameplayContext { dt_ms, inputs, &mut Stats }`。
   需要更多 → 改契约 → 先写 ADR。

3. **加一个玩法不修改 `rustory-core`。**
   如果做不到，说明契约设计错了，先修契约。

4. **引用完整性在加载期检查，不在使用期。**
   悬空跳转、未映射的结果键、循环 goto，都必须在第一帧之前报错。

5. **渲染器不认识游戏概念。**
   `rustory-render` 里出现 `Story`、`Actor`、`Choice` 就是泄漏。
   它只认识 `Frame`/`Quad`/`TextItem`。

6. **逐字显示按 grapheme cluster 推进，不按 `char`。**
   `char` 会把 emoji 和组合字符切碎。

---

## 5. 模块速查

```
crates/rustory-core/src/
  ids.rs          NodeId / ActorId / SystemId / OutcomeKey（newtype，防混用）
  story.rs        Story / Node / Beat / Exit / Choice / GameplaySpec
  stage.rs        Stage（背景 + 在台上的角色），显式字段，不用动态组件表
  narrative.rs    状态机：cursor、beat 索引、逐字进度、Mode
  event.rs        NarrativeEvent、TickReport
  gameplay.rs     GameplaySystem trait、GameplayContext、Stats、Registry
  error.rs        CoreError

crates/rustory-script/src/lib.rs
  parse_story / load_story，错误带行列号

crates/rustory-render/src/
  frame.rs        Frame / Quad / TextItem / Rect / Rgba / Uv
  image.rs        Rgba8Image（PNG 读写、纯色、渐变）
  renderer.rs     wgpu 管线、纹理上传、离屏读回

crates/rustory-gameplay-timer/src/lib.rs
  限时选择：TimerConfig、TimedChoice

apps/rustory/src/
  main.rs         CLI 分派：probe / check / transcript / gen-assets / shot / play
  driver.rs       click 语义、注册表、纹理加载、transcript
  scene.rs        Narrative → Frame 的唯一翻译层
  play.rs         winit 事件循环、surface、输入映射
  assets.rs       程序化占位美术
```

---

## 6. 为什么没有通用 ECS

HANDOFF 原本计划用 Bevy ECS，理由是「玩法 = 独立 System」。
实际实现下来，这个类比不成立：

- VN 的状态形状是**固定且小**的：一个舞台（背景 + 几个角色）、一条游标、一份数值。
- ECS 的收益在「大量异构实体 + 未知组合」。VN 两样都没有。
- ECS 的代价是真实的：实体重映射让存档复杂化（见 ADR 0006），
  `Query` 的隐式访问集让「玩法能碰什么」变得不可枚举。

所以现在的做法是：**`Stage` 用显式结构体字段，`Stats` 用窄字典，
玩法用 trait 对象注册。** 三样都可枚举、可序列化、可测试。

如果将来真出现「成千上万个同类实体需要批处理」的玩法（例如弹幕），
再为那个玩法单独引入一个专用存储，而不是给全引擎套 ECS。

---

## 7. 测试策略

| 层 | 怎么测 | 需要 GPU？ |
|---|---|---|
| 状态机 | 集成测试，纯内存 | 否 |
| 剧本格式 | 正例 + 反例（悬空引用、语法错误） | 否 |
| 玩法系统 | 构造 `GameplayContext` 直接 step | 否 |
| 布局 | 断言 `Frame` 里的矩形坐标 | 否 |
| 渲染 | 离屏渲染 → 读回像素 → 比对相对亮度 | **是** |
| 窗口 | 实际运行，无诊断输出 | 是 |

渲染测试在没有适配器的机器上**跳过**而非失败：缺 GPU 是环境事实，不是回归。

---

## 8. 已知缺口

- **没有存档**（ADR 0006 是提案）。
- **没有音频**。
- **没有玩法读叙事状态的通道**：玩法不知道现在是第几轮对话，只能靠配置传参。
  需要时应加只读快照，而不是开放 `Narrative`。
- **`--advance` 的语义对用户不直观**：它按「一次交互」计数，
  而一次交互可能跨两个状态（完成当前行 + 前进一个 beat）。
- **玩法配置错误在运行到该节点时才暴露**，不是加载时。
