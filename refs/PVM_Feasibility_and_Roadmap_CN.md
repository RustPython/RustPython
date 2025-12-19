# 基于现有 RustPython 代码库实现 PVM 的可行性与实现路径

**目标**：基于当前 RustPython 代码库，实现 Cowboy 规划文档中的 PVM（确定性 Python VM + Actor/消息 + 可验证链下计算的编程模型）。

**分析来源**：
- refs/Cowboy_An_Actor-Model_Layer1 with Verifiable_Off-Chain_Compute_CN.md
- refs/Cowboy_An_Actor-Model_Layer1 with Verifiable_Off-Chain_Compute_EN.md
- refs/Cowboy_An_Actor-Model_Layer1 with Verifiable_Off-Chain_Compute(Sugguestion-SDK Ergonomics)_v2.md
- refs/Cowboy_An_Actor-Model_Layer1 with Verifiable_Off-Chain_Compute(Sugguestion-SDK Ergonomics)_v3_EN.md

---

## 结论（可行性）

**可行但不“开箱即用”**。RustPython 已具备完整的 Python 解释器、编译器与标准库骨架，适合作为 PVM 的运行时基础。但 Cowboy 的 PVM 需要**强确定性、强资源计量、严格的运行时约束**和**Actor/Continuation 语义**，这些目前在 RustPython 中不存在或仅部分支持，需新增一层“链上执行环境”的系统功能与编译期约束。

简单地说：**RustPython 解决“能运行 Python”，PVM 需要解决“可共识地、可计量地运行 Python”**。

---

## 现有代码库可复用能力（要点）

### 可直接复用或轻改的部分
- **解释器与字节码执行器**：`crates/vm`（执行引擎）与 `crates/compiler`（编译器）是 PVM 的核心基础。
- **可控 JIT**：JIT 是 feature flag（`crates/vm/src/builtins/function.rs`），可在 PVM 构建配置中彻底关闭。
- **Hash Seed 控制**：`src/settings.rs` 支持 `PYTHONHASHSEED` 固定值，有利于确定性哈希。
- **内置模块体系**：可在 `crates/vm/src/stdlib` 内新建/替换系统模块实现 SDK 与系统 Actor。

### 当前缺失或不满足 PVM 约束的部分
- **无 CBOR 体系**：全库未发现 CBOR 实现，无法满足“跨块状态序列化 + Canonical CBOR”约束。
- **无资源计量/燃料系统**：执行器中未见 instruction-level gas/cycles 计量。
- **标准库非确定性来源过多**：`Lib/` 内包含时间、随机、线程、IO、网络等大量非确定性模块。
- **浮点全为硬件 float**：`crates/vm/src/builtins/float.rs` 依赖 `f64`，与“SoftFloat”要求不符。
- **对象标识/哈希不稳定**：部分哈希退化路径使用对象 id（如 NaN），这在共识场景中不可接受。

---

## 实现路径建议（分阶段）

### 阶段 0：PVM 构建与运行时基线
1. 建立 `pvm` 构建 profile：
   - 禁用 JIT（不启用 `jit` feature）。
   - 禁用线程（不启用 `threading` feature）。
2. 固定哈希种子与环境：
   - 启动时强制 `PYTHONHASHSEED` 为固定值（不允许 random）。
3. 建立“受限 stdlib”列表：
   - 仅开放确定性、安全模块（`math` 需替换 float 依赖，`time/random/os/socket/subprocess/ctypes` 默认禁用）。

### 阶段 1：确定性运行时与系统 API
1. **确定性系统模块**：
   - `time` → `block_height`/`block_timestamp`（由链提供）。
   - `random` → VRF/链上伪随机接口。
   - `hash` 与 `id()` → 明确禁用或强制确定性定义。
2. **Actor 系统调用层**：
   - 引入 `cowboy_sdk` 内建模块，暴露 `call() / send() / await` 等原语。
   - Actor 存储 API（KV、配额、租金）与邮箱接口。

### 阶段 2：资源计量（Cycles/Cells）
1. **字节码指令计量**：
   - 在 `crates/vm/src/frame.rs` 指令执行循环插入燃料消耗点。
2. **存储/序列化计量**：
   - 读写 Actor 存储按字节计费（Cells）。
   - 序列化（CBOR）大小计费。
3. **跨 Actor 调用深度与递归深度限制**：
   - 已有递归限制可复用，但需与 call 深度上限对齐（文档要求 32）。

### 阶段 3：Continuation 编译与状态机
1. **语法与编译期限制**：
   - `await` 点数量、`@bounded_loop` 等限制需要在编译期静态检查。
2. **AST/Bytecode 变换**：
   - 将 async/await 编译为显式 FSM（状态机），生成 `__resume` 入口。
3. **状态捕获与 Guard**：
   - `capture()` 强制显式声明跨块变量。
   - `guard_unchanged` 在恢复时验证存储哈希。

### 阶段 4：CBOR 与可验证数据模型
1. **Canonical CBOR 编码/解码**：
   - 作为链上状态与消息序列化标准。
2. **类型系统适配**：
   - `SoftFloat`、`ordered_set` 等 SDK 类型实现与运行时检查。
3. **验证构建器**：
   - 实现 `Verify.builder()` 语义，产生确定性 JSON/CBOR 规格。

### 阶段 5：Runner 与链下验证接口
1. Runner 任务请求与回调处理。
2. 响应结果的验证（N-of-M、TEE、ZK 接口预留）。

---

## 关键技术难点（必须列出）

1. **确定性执行的全域治理**
   - 浮点（`f64`）需要替换为软浮点实现；否则跨平台不一致。
   - `hash()`/`id()`、集合迭代顺序等需严格规范或禁用。
   - `random/time/os` 等环境依赖必须替换为链提供接口。

2. **字节码级资源计量**
   - RustPython 目前无“指令燃料”机制，需重构执行循环。
   - 计量必须与语义一致，不能因优化或平台差异引入偏差。

3. **Continuation 编译与语义约束**
   - 需要将 async/await 编译为 FSM，处理分支、异常与 bounded loop。
   - 捕获变量 CBOR 化、状态大小限制、状态清理均需严格实现。

4. **CBOR Canonical 化**
   - 现有代码中无 CBOR，需新增高质量实现并确保序列化稳定。
   - 所有跨块状态、消息、返回值必须 CBOR 化，且在错误路径一致。

5. **Actor 模型与调度器**
   - 需要提供 mailbox、消息队列、定时器与 call 深度限制。
   - 需要“恰好一次”消息语义与防重放机制。

6. **标准库裁剪与沙箱**
   - `Lib/` 需要建立白名单并定制替换版模块。
   - 禁止文件系统、网络、进程、FFI、线程、系统时间等不确定性源。

7. **软浮点与类型替换成本**
   - `float` 在解释器里是核心类型，替换为 SoftFloat 会牵涉大量内建操作。
   - 还需处理 `complex`、`decimal` 等依赖链。

8. **异常与回滚语义的一致性**
   - Actor 调用链的原子回滚必须与 call/send/await 语义一致。
   - 异常传播与资源计量扣费需定义清晰的顺序与边界。

9. **跨语言 SDK 语法糖与编译期约束**
   - `ActorRef` / `@runner.continuation` / `@bounded_loop` 等需要编译期转换与检查。
   - 需要工具链或编译插件支持（可能要在 RustPython 编译器层新增 pass）。

10. **性能与成本控制**
   - 软浮点 + CBOR + 状态机转换可能引入性能开销。
   - 需要在确定性与性能之间明确取舍。

---

## 可交付的最小版本（建议）

1. **确定性 Python 子集**
   - 禁用随机、时间、IO、线程、FFI。
   - 固定 hash seed 与对象行为。
2. **基础 Actor 模型**
   - `call()` / `send()` 语义，有限深度与 gas。
3. **简单 Continuation**
   - 仅支持顺序 await（<= 2），无 loop/branch。
4. **CBOR 与存储**
   - Actor 状态序列化、消息序列化可用。

该最小版本可以用来验证“共识级确定性 + Actor 交互 + 资源计量”的可行性，再逐步引入复杂的 SDK ergonomics。

---

## 与现有代码的对接建议

### 可能的技术切入点
- `crates/vm/src/frame.rs`：指令级 gas/cycles 计量。
- `crates/vm/src/stdlib`：替换/新增 SDK 与系统模块。
- `src/settings.rs`：强制固定 `PYTHONHASHSEED`。
- `crates/compiler`：加入 async->FSM 的编译 pass。

### 建议新增的 crate/模块
- `crates/pvm`：PVM 运行时封装（Actor、消息、计量、CBOR）。
- `crates/cowboy-sdk`：SDK 的 Python 侧实现与编译期工具。

