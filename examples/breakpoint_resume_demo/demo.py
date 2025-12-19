from __future__ import annotations

from pathlib import Path

import rustpython_checkpoint as rpc

# 断点文件路径：保存为字符串，确保可序列化
CHECKPOINT_PATH = str(Path(__file__).with_suffix(".rpsnap"))

# 第一段逻辑：准备变量和上下文
print("[run] phase=init")
user = "alice"
amount = 120
items = [f"item_{idx}" for idx in range(3)]
analysis = {"score": 0.6, "summary": "score=0.6"}
print(f"[run] user={user} amount={amount} items={items} analysis={analysis}")

# 断点 1：必须是“独立语句”，不能写在赋值/条件表达式里
# 运行到此处 RustPython 会保存 VM 状态并退出进程
rpc.checkpoint(CHECKPOINT_PATH)

# 断点恢复后重新导入模块，确保后续断点可用
import rustpython_checkpoint as rpc

# 第二段逻辑：继续使用上一次保存的变量
print("[run] phase=after_checkpoint_1")
processed = [f"{user}:{item}" for item in items]
total = amount + len(processed)
print(f"[run] processed={processed} total={total}")

# 断点 2：再次保存状态并退出，下一次继续向下执行
rpc.checkpoint(CHECKPOINT_PATH)

# 断点恢复后准备清理工具
import os

# 第三段逻辑：从第二个断点恢复后执行
print("[run] phase=after_checkpoint_2")
receipt = {
    "user": user,
    "total": total,
    "processed": processed,
    "status": "ok",
}
print(f"[run] receipt={receipt}")

# 清理断点文件，方便下次从头开始
if os.path.exists(CHECKPOINT_PATH):
    os.remove(CHECKPOINT_PATH)
print("[run] done")
