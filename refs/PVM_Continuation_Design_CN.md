# PVM Continuation 设计草案（Actor 与 Runner）

**目的**：定义 Actor↔Actor 与 Actor↔Runner 的 continuation 机制实现方案，覆盖 SDK API、编译期约束、CBOR schema 与运行时数据结构。

---

## 1. SDK API 草案（Python 侧）

### 1.1 调用原语
- `call(target: str, method: str, args: dict, cycles_limit: int) -> Any`
  - 同区块同步执行，返回值必须 CBOR-safe。
- `send(target: str, message: dict) -> None`
  - 异步投递至下一区块。

### 1.2 Continuation 相关
- `capture() -> Ctx`
  - 返回 dict-like 对象，仅允许写入 CBOR-safe 类型。
- `@runner.continuation(timeout_blocks: int = 0, guard_unchanged: list[str] = [])`
  - async 函数编译为 FSM；跨块执行。
- `@actor.continuation(timeout_blocks: int = 0, guard_unchanged: list[str] = [])`
  - Actor 间异步请求-响应。
- `ActorRef(address: str)`
  - `ActorRef.async_<method>(*args, **kwargs)` 编译为 send + resume。

### 1.3 Runner API
- `runner.llm(prompt: str, response_model=None, verification=None, timeout_blocks=0, tee_required=False)`
- `runner.http(url: str, method="GET", headers=None, body=None, timeout_blocks=0, retry_policy=None)`

### 1.4 确定性异常
- `StateConflictError`（guard 失败）
- `ContinuationTimeoutError`
- `DeterministicValidationError`（Schema/CBOR 校验失败）
- `LoopBoundExceeded`

---

## 2. 编译期约束（强制）

1. `await` 点数量上限（建议 8，最小版本可先 2）
2. 循环中 `await` 必须通过 `@bounded_loop(max_iterations=...)`
3. 禁止嵌套函数中 `await`
4. 禁止递归 `await`
5. `capture()` 必须显式声明跨 await 的变量
6. `guard_unchanged` 的 key 必须是 storage 的稳定 key

---

## 3. Continuation 编译形态（FSM）

原始 async 函数：
```python
@runner.continuation
async def f(self, msg):
    ctx = capture()
    ctx.a = await runner.llm("step1")
    ctx.b = await runner.llm(f"step2: {ctx.a}")
    return ctx.b
```

编译后（示意）：
```python
def f(self, msg):
    cid = _new_cid(self, "f")
    _save_cont(cid, state=0, ctx={}, guard=...)
    send(RUNNER, job=..., cid=cid, reply="f__resume")

def f__resume(self, reply):
    st = _load_cont(reply.cid)
    if st.state == 0:
        st.ctx["a"] = reply.result
        _save_cont(reply.cid, state=1, ctx=st.ctx, guard=st.guard)
        send(RUNNER, job=..., cid=reply.cid, reply="f__resume")
        return
    if st.state == 1:
        st.ctx["b"] = reply.result
        _delete_cont(reply.cid)
        return st.ctx["b"]
```

---

## 4. CBOR Schema（消息与状态）

### 4.1 Continuation State
```text
CBOR Map {
  "state": uint,
  "ctx": map<string, cbor_value>,
  "guard": map<string, bytes32>,
  "created_block": uint,
  "timeout_block": uint,
  "checksum": bytes32
}
```

### 4.2 Runner Job
```text
CBOR Map {
  "kind": "runner_job",
  "job_type": "llm" | "http" | "custom",
  "payload": map,
  "cid": bytes32,
  "reply_to": bytes20,
  "reply_handler": string,
  "timeout_block": uint,
  "verification": map,
  "tee_required": bool
}
```

### 4.3 Runner Result
```text
CBOR Map {
  "kind": "runner_result",
  "cid": bytes32,
  "status": "ok" | "error",
  "result": cbor_value,
  "proof": map
}
```

### 4.4 Actor Async Call
```text
CBOR Map {
  "kind": "actor_async_call",
  "cid": bytes32,
  "method": string,
  "args": map,
  "reply_handler": string
}
```

---

## 5. `capture()` 允许的 CBOR-safe 类型（白名单）

- `None`, `bool`, `int`, `bytes`, `str`
- `list`（成员需 CBOR-safe）
- `dict`（key 必须为 `str`，value 必须 CBOR-safe）
- `SoftFloat`（软件浮点类型）
- `ordered_set`（需序列化为有序数组）

禁止：
- 函数/闭包/生成器
- 文件句柄、socket、线程对象
- 任意含非确定性内部状态的对象

---

## 6. Guard 机制

### 6.1 Decorator Guard
`@runner.continuation(guard_unchanged=["k1","k2"])`
- 保存 `hash(cbor(storage["k1"]))`
- 恢复时重新计算，若不同抛 `StateConflictError`

### 6.2 Object Guard
`self.storage.guard("balance")`
- 返回 `GuardedValue`，访问 `.value` 时触发校验

---

## 7. `Verify.builder()` 的 CBOR Schema（草案）

```text
CBOR Map {
  "mode": "none" | "economic_bond" | "majority_vote" | "structured_match" | "deterministic" | "semantic_similarity",
  "runners": uint,
  "threshold": uint,
  "checks": [
    { "kind": "json_schema_valid", "schema": map },
    { "kind": "numeric_tolerance", "field": string, "tolerance": SoftFloat },
    { "kind": "exact_match" },
    { "kind": "structured_match", "fields": [string] },
    { "kind": "no_prompt_leak" },
    { "kind": "custom", "actor": bytes20, "method": string }
  ]
}
```

---

## 8. `@bounded_loop` 编译约束（伪代码）

```text
if loop contains await:
    require @bounded_loop(max_iterations=N)
    require N is compile-time constant
    insert iteration counter
    if counter > N: raise LoopBoundExceeded
```

---

## 9. Rust 侧结构草图（示意）

```rust
pub struct ContinuationState {
    pub state: u32,
    pub ctx: BTreeMap<String, CborValue>,
    pub guard: BTreeMap<String, [u8; 32]>,
    pub created_block: u64,
    pub timeout_block: u64,
    pub checksum: [u8; 32],
}

pub enum RunnerJobType {
    Llm,
    Http,
    Custom(String),
}

pub struct RunnerJob {
    pub job_type: RunnerJobType,
    pub payload: CborValue,
    pub cid: [u8; 32],
    pub reply_to: [u8; 20],
    pub reply_handler: String,
    pub timeout_block: u64,
    pub verification: CborValue,
    pub tee_required: bool,
}

pub struct RunnerResult {
    pub cid: [u8; 32],
    pub status: RunnerStatus,
    pub result: CborValue,
    pub proof: CborValue,
}

pub enum RunnerStatus {
    Ok,
    Error,
}

pub struct ActorAsyncCall {
    pub cid: [u8; 32],
    pub method: String,
    pub args: CborValue,
    pub reply_handler: String,
}
```

---

## 10. 最小实现顺序建议

1. Continuation State 存储 + `capture()` 白名单
2. FSM 编译（顺序 await）
3. Runner Job/Result CBOR 协议
4. Guard 校验 + 超时处理
5. Actor↔Actor async

