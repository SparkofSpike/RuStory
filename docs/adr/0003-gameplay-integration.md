# ADR 0003 — 玩法接入：trait + 注册表 + 不透明配置

- **状态**：已采纳并已实现
- **日期**：2026-10-26
- **对应待决问题**：HANDOFF §4.3 第 2 问

---

## 背景

「加新玩法是加一根纬线」是项目的第一句话。这句话要么是真的，要么是标语。
区别在于：**加一个玩法系统时，需要改哪些文件？**

如果答案是「核心的 enum 加一个 variant」，那标语就是假的——因为每加一个玩法
都在改核心，核心迟早变成所有玩法的杂物间。

## 选项

### A. 核心 enum 列出所有玩法

```rust
pub enum GameplaySpec {
    Debate { rounds: u32 },
    Puzzle { grid: Vec<u8> },
}
```

- 优点：编译期完全类型安全。
- 缺点：**每加一个玩法都要改核心**，且核心必须依赖所有玩法的配置类型。
  直接违背项目目标。

### B. 完全动态：`HashMap<String, Box<dyn Any>>`

- 优点：核心不知道任何玩法。
- 缺点：配置形状靠运行时碰运气，正是 HANDOFF §1 点名的结构性问题。

### C. trait 对象注册表 + 不透明配置切片

```rust
pub trait GameplaySystem {
    fn id(&self) -> SystemId;
    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError>;
    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey>;
    fn cancel(&mut self);
    fn prompt(&self) -> Option<String> { None }
}
```

核心只认三件事：系统 id、一段它**不解析**的配置、一个**它不解释**的结果键。
出口由剧本的 `outcomes` 表决定。

## 决策

**采用 C。**

## 理由

1. **类型安全被下推到玩法的边界之内。** 核心侧是弱类型（`ron::Value`），
   但每个玩法在 `begin` 里立刻反序列化成自己的强类型配置，并且**校验失败就不接管**。
   类型安全的损失被限制在一个方法里，而不是扩散到整个核心。
2. **出口语义属于剧本，不属于代码。** `correct` 该跳到哪，是这一场戏的事。
   同一个系统在不同场景可以有完全不同的后果，不需要改代码。
3. **类型擦除只有一处，宽度只有四个方法。** 这不是「动态形状拼凑」——
   被擦除的表面是可枚举的、有文档的、被测试覆盖的。
4. **加法式扩展可被机械验证。** 见下方验证一节。

## 后果

**得到的**

- 新增玩法 = 新建一个 crate + 注册一行。核心零改动。
- 玩法可以独立测试：构造 `GameplayContext` 就能跑，不需要故事、窗口或 GPU。
- 玩法之间互不感知，顺序由剧本决定。

**付出的**

- 配置错误在**运行到该节点时**才暴露（`begin` 失败），不是加载时。
  缓解：`rustory check` 会列出脚本调用的系统是否已注册；将来可加「试运行所有
  `begin`」的预检。
- `ron::Value` 泄漏进了核心的公开 API。接受：它是数据格式类型，不是玩法类型。
- 注册表是 `BTreeMap<SystemId, Box<dyn GameplaySystem>>`，
  同一个 id 只能有一个实例。若将来需要并行多个实例，才引入「系统实例池」。

**明确不做的**

- 不提供 `&mut World` 之类的万能后门。玩法只能拿到 `GameplayContext`
  （时间、输入、`Stats`）。需要更多共享状态就是一次契约扩张，必须走 ADR。

## 验证

`rustory-gameplay-timer` 是这个决策的实证：

- 它独立成 crate，只依赖 `rustory-core`。
- `rustory-core` 里没有任何一处提到 timer、选项或正确性。
- 接入方式是 `driver.rs` 里的一行 `registry.register(Box::new(TimedChoice::new()))`。
- 它不感知舞台、故事或渲染——`GameplayContext` 里根本没有这些。

可以说：**加这根纬线，没有动那块布。**
