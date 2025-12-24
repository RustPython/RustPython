# 断点续运行演示（VM 级）

这个演示使用 RustPython 的 VM 级断点/恢复原型：运行到断点行时保存虚拟机状态并退出进程；再次运行时加载断点数据，直接从断点行之后继续执行。

## 运行方式

第一次运行（触发断点 1 并退出）：

```
./target/release/rustpython examples/breakpoint_resume_demo/demo.py
```

第二次运行（从断点 1 恢复，继续到断点 2 并退出）：

```
./target/release/rustpython --resume examples/breakpoint_resume_demo/demo.rpsnap examples/breakpoint_resume_demo/demo.py
```

第三次运行（从断点 2 恢复，执行完毕并清理断点文件）：

```
./target/release/rustpython --resume examples/breakpoint_resume_demo/demo.rpsnap examples/breakpoint_resume_demo/demo.py
```

## 测试程序

可直接运行自动化测试脚本验证断点续跑是否正常：

```
./target/release/rustpython examples/breakpoint_resume_demo/test_checkpoint.py
```

如需指定 rustpython 可执行文件路径：

```
./target/release/rustpython examples/breakpoint_resume_demo/test_checkpoint.py --bin /path/to/rustpython
```

## 断点文件

断点状态保存在：

```
examples/breakpoint_resume_demo/demo.rpsnap
```

这是一个由 `marshal` 序列化的二进制文件，请不要手动编辑。

## 限制说明（原型能力边界）

当前原型是“最小可用”的 VM 级断点续跑，主要限制如下：

- 仅支持脚本顶层（模块级）代码；不支持函数/方法内部断点。
- 断点位置必须是“独立语句”（例如 `checkpoint()` 单独一行），不能放在赋值或条件表达式中。
- 断点时不能处于循环/try/finally 等控制流块的内部（即块栈必须为空）。
- 断点只保存可被 `marshal` 序列化的简单类型（如 int/str/list/dict 等）。模块对象、类、文件句柄等不会被保存，需要在断点恢复后重新导入或重建。

这些限制在 README 中明确，是为了让示例完整呈现“同一代码块中断点续跑”的效果。
