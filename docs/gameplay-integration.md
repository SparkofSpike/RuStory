# 玩法接入指南

本文说明**怎么加一个新的玩法系统**，以及为什么这些步骤不会碰到叙事核心。

如果你只想看结论：新建一个 crate，实现 `GameplaySystem`，在 `driver.rs` 里
注册一行，在剧本里写一个 `Gameplay` 出口。**`rustory-core` 一行都不用改。**

---

## 1. 契约长什么样

```rust
pub trait GameplaySystem {
    fn id(&self) -> SystemId;
    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError>;
    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey>;
    fn cancel(&mut self);
    fn prompt(&self) -> Option<String> { None }
}
```

四个方法，职责分明：

- `begin`：拿到剧本写的配置，**自己**解析成强类型，失败就报错（不接管）。
- `step`：推进一帧。返回 `Some(key)` 表示结束并交回控制权。
- `cancel`：故事被中断（关窗口、读档跳到别处），必须把自己清干净。
- `prompt`：给玩家看的提示，默认不显示。

`step` 能拿到的东西**只有这些**：

```rust
pub struct GameplayContext<'a> {
    pub dt_ms: u32,
    pub inputs: &'a [GameplayInput],   // Confirm | Select { index }
    pub stats: &'a mut Stats,          // BTreeMap<String, i32>
}
```

舞台、故事图、渲染器、窗口——都不在里面。这不是约定，是类型层面的隔离。

---

## 2. 五步接入

### 步骤 1：新建 crate

```
crates/rustory-gameplay-<name>/
├── Cargo.toml
└── src/lib.rs
```

```toml
[package]
name = "rustory-gameplay-<name>"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "RuStory gameplay system: <一句话>"

[dependencies]
rustory-core = { workspace = true }
serde = { workspace = true }
ron = { workspace = true }
thiserror = { workspace = true }
```

在根 `Cargo.toml` 的 `members` 和 `[workspace.dependencies]` 各加一行。

### 步骤 2：定义配置

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct MyConfig {
    pub rounds: u32,
    #[serde(default)]
    pub prompt: Option<String>,
}

impl MyConfig {
    pub fn from_value(value: Option<&ron::Value>) -> Result<Self, String> {
        let value = value.ok_or("config is required")?;
        let config: Self = value.clone().into_rust::<Self>()
            .map_err(|err| err.to_string())?;
        if config.rounds == 0 {
            return Err("rounds must be positive".into());
        }
        Ok(config)
    }
}
```

**校验放在这里。** 一个坏配置应该在接管时就被拒绝，而不是玩到一半崩掉。

### 步骤 3：实现 trait

```rust
pub struct MySystem {
    config: Option<MyConfig>,
    remaining: u32,
}

impl GameplaySystem for MySystem {
    fn id(&self) -> SystemId {
        SystemId::from("my.system")
    }

    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError> {
        // 先校验，后改状态：失败时不能留下半初始化
        let parsed = MyConfig::from_value(config).map_err(|message| CoreError::GameplayFailed {
            system: "my.system".to_owned(),
            message,
        })?;
        self.remaining = parsed.rounds;
        self.config = Some(parsed);
        Ok(())
    }

    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        // Guard: `step` may be called before `begin` completed, and there is
        // nothing to do until it has.
        self.config.as_ref()?;

        // 输入优先于时钟：同一帧的选择算数
        for input in ctx.inputs {
            if let GameplayInput::Confirm = input {
                ctx.stats.add("my.system.confirms", 1);
                return Some(OutcomeKey::from("confirmed"));
            }
        }

        self.remaining = self.remaining.saturating_sub(ctx.dt_ms);
        if self.remaining == 0 {
            return Some(OutcomeKey::from("expired"));
        }
        None
    }

    fn cancel(&mut self) {
        self.config = None;
        self.remaining = 0;
    }

    fn prompt(&self) -> Option<String> {
        self.config.as_ref()?;
        Some(format!("{}ms left", self.remaining))
    }
}
```

### 步骤 4：注册（一行）

`apps/rustory/src/driver.rs`：

```rust
pub fn default_registry() -> GameplayRegistry {
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(TimedChoice::new()));
    registry.register(Box::new(MySystem::new()));   // ← 就这一行
    registry
}
```

### 步骤 5：在剧本里用

```ron
"my_scene": (
    beats: [ Say(text: "Something is about to happen.") ],
    exit: Gameplay(spec: (
        system: "my.system",
        config: Some((rounds: 3000, prompt: Some("Press confirm"))),
        outcomes: {
            "confirmed": "you_did_it",
            "expired": "too_late",
        },
    )),
),
```

注意 `outcomes` 在**剧本**里。同一个系统在不同场景可以有不同后果，
不需要改代码。

---

## 3. 为什么这不会碰核心

| 关心的事 | 谁负责 | 核心知道吗 |
|---|---|---|
| 配置字段 | 玩法自己的 `MyConfig` | 不知道（只当不透明值转交） |
| 结果含义 | 剧本的 `outcomes` 表 | 不知道（只当字符串键查表） |
| 内部状态 | 玩法自己的字段 | 不知道 |
| 何时结束 | 玩法返回 `Some(key)` | 只知道「结束了」 |

核心新增的唯一东西是：**如果这个玩法被注册了，就能跑**。
这就是「加法式扩展」的具体含义。

---

## 4. 必须遵守的几条

### 不要试图拿到更多状态

`GameplayContext` 里没有舞台、没有故事图。别加后门（比如往 `Stats` 里塞
序列化后的舞台）。

真需要的话，正确做法是：
1. 写 ADR 说明为什么这个玩法必须读舞台；
2. 加一个**只读**的快照类型，而不是 `&mut Narrative`；
3. 更新本文。

### `cancel` 必须真的清干净

故事会被中断：窗口关闭、读档跳到别的节点、开发者热重载。
如果 `cancel` 留了残状态，下一次 `begin` 会继承脏数据——这类 bug 极难查。

### 输入要在时钟之前处理

玩家在计时器归零的同一帧按下确认，算他成功。反过来会让玩家觉得「明明按到了」。

### 数值写进 `Stats` 时带命名空间

```rust
ctx.stats.add("timed_choice.correct", 1);   // 好
ctx.stats.add("correct", 1);                // 会和别的玩法撞
```

`Stats` 的键是字符串，编译器不会帮你查拼写。

### 不要在 `step` 里做重活

它每帧调用一次。解析配置放 `begin`，分配放 `begin`，
`step` 只做算术和状态转移。

---

## 5. 测试怎么写

玩法测试**不需要故事、窗口或 GPU**。直接构造上下文：

```rust
#[test]
fn confirms_before_the_deadline() {
    let mut system = MySystem::new();
    system.begin(Some(&config)).expect("valid config");
    let mut stats = Stats::default();

    let outcome = system.step(&mut GameplayContext {
        dt_ms: 16,
        inputs: &[GameplayInput::Confirm],
        stats: &mut stats,
    });

    assert_eq!(outcome, Some(OutcomeKey::from("confirmed")));
}
```

建议覆盖：

- 每个可能的结果键至少一次；
- 边界：归零那一帧、非法索引、空配置；
- `cancel` 之后 `begin` 能重新开始；
- 配置校验失败时**没有**改动内部状态。

参考 `crates/rustory-gameplay-timer/src/lib.rs` 的 9 个测试。

---

## 6. 反例：不该这样写

```rust
// ✗ 核心必须知道这个玩法
pub enum GameplaySpec {
    MySystemThing { rounds: u32 },
}
```

```rust
// ✗ 动态形状，运行时才知道对不对
ctx.stats_raw.insert("anything", Box::new(vec![1,2,3]));
```

```rust
// ✗ 让玩法直接改叙事
fn step(&mut self, narrative: &mut Narrative) { ... }
```

```rust
// ✗ 在 step 里解析配置（每帧一次）
fn step(&mut self, ctx: &mut GameplayContext) {
    let config: MyConfig = ron::from_value(self.raw.clone()).unwrap(); // 每帧！
}
```

前三个破坏架构承诺，第四个只是慢——但慢到卡帧时，你会以为是渲染的问题。

---

## 7. 现有示例

`crates/rustory-gameplay-timer` 是完整参考实现：

- 配置：`TimerConfig { prompt, duration_ms, options, correct }`
- 结果：`correct` / `wrong` / `timeout`
- 覆盖的行为：答案优先于时钟、非法索引忽略、配置校验前置、`cancel` 清空。
- 9 个单元测试，零外部依赖。

演示剧本 `stories/demo.ron` 的 `honest` 节点展示了它在真实场景里的用法。
