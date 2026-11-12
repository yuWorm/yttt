# 移动端接管后的按键栏

分支 `feat/mobile-key-bar`，worktree `.worktrees/mobile-key-bar`，基于 master 5954658。

设计 §2：接管后，提示换成按键栏（Esc、Tab、Ctrl、方向键、回车、键盘）和“交还电脑”。手机软键盘打不出 Esc、方向键和 Ctrl 组合键，没有按键栏就没法操作 Agent CLI（比如 Claude Code 的确认菜单、Ctrl-C 中断）。

- **iOS**：SwiftTerm 自带按键栏 `TerminalAccessory`，包括 esc、ctrl、tab、方向键、~ | / - 等。它的方向键按终端自己的光标模式编码，这个模式由我们的 VT 流同步，所以沿用它，只做验证。
- **Android**：termlib 没有按键栏，要自己做。

每个任务先写失败的测试，再实现，测试通过后提交。编译用 `CARGO_PROFILE_DEV_DEBUG=line-tables-only`，合并后清理构建缓存。

## 任务 1：按键编码（Rust，两端共用）

`crates/yttt-mobile-core/src/keys.rs`：

```rust
pub enum TerminalKey { Escape, Tab, ShiftTab, Enter, Backspace, Up, Down, Left, Right }
pub fn encode(key: TerminalKey, app_cursor: bool) -> &'static [u8]
```

- 编码规则：
  - Esc `\x1b`，Tab `\t`，Shift-Tab `\x1b[Z`，回车 `\r`，退格 `\x7f`；
  - 方向键：普通模式 `\x1b[A..D`，应用光标模式（DECCKM，`?1h`）`\x1bOA..D`。
- 单元测试覆盖每个键，以及两种光标模式。
- `MobileSession::send_key(session_id, key)`：
  - 从最新 viewport 的 `modes.bits` 读 `APP_CURSOR`，编码后走 `send_input`；
  - 和普通输入一样，只在接管时有效。
- 集成测试 `keys_reach_the_program_as_it_expects_them`，真 Host：
  - 终端运行一个脚本：`stty raw -echo` 后读 3 个字节，用 `od` 打印十六进制；接着输出 `\033[?1h` 进入应用光标模式，再读 3 个字节、打印；
  - 手机接管后发两次“上”；
  - 断言第一次打印 `1b 5b 41`，第二次打印 `1b 4f 41`。
- 发第二个键之前，要等 viewport 里出现应用光标模式，否则这个测试就测不到模式切换。

## 任务 2：FFI

`HostSession.sendKey(sessionId, key)`，`TerminalKey` 为 uniffi Enum。

## 任务 3：Android 按键栏

- 接管后，在终端下方、软键盘上方显示一行按键。它跟着 `imePadding` 一起上移。
  - 顺序：Esc、Tab、Ctrl、←、↓、↑、→、⏎、⌨。
  - 可以横向滚动，容纳 Shift-Tab、退格。
- **Ctrl 是一次性修饰键**：点亮后，软键盘打出的下一个字母按 Ctrl 发送（`a`–`z` 和 `@[\]^_` 映射到 `0x00`–`0x1f`），然后自动熄灭；再点一次可以取消。这部分是纯 Kotlin 的 `CtrlKey`，写单元测试。
- ⌨ 用来显示或隐藏软键盘。
- “接管终端”提示在接管后隐藏，“交还电脑”保留在顶部。

## 任务 4：iOS 验证

- 确认接管后 SwiftTerm 的按键栏出现。
- 在模拟器上用自动化参数接管上面那个读字节的脚本，模拟器的硬件键盘可以发方向键：`xcrun simctl io` 不能发键，就用 SwiftTerm 的 `send` 路径。
- 如果自动化做不到，就只验证按键栏显示出来，编码正确性交给任务 1 的集成测试。SwiftTerm 的编码由它自己负责。

## 任务 5：模拟器验证（Android）

在 spike Host 的 shell 里：

- 接管，输入 `echo abc`，按 ← 两次再输入 `X`，回车，屏幕显示 `aXbc`；
- 输入 `sleep 30`，回车，再按 Ctrl 和 `c`，提示符回来；
- 输入 `ec`，按 Tab，补全成 `echo`。

截图存本地 scratchpad，不上传。

## 任务 6：结论

更新移动端设计 §2、§4，在本文末尾写执行记录。
