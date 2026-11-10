# QuickExit — 副本退出快捷鍵

按一個自訂快捷鍵，程式會自動「喚出滑鼠 → 移到右上角退出按鈕 → 點擊」，不用再手動按 Ctrl 加滑鼠點。

## 使用方式

1. 把 `QuickExit.exe` 放在一個**可以寫入**的資料夾（例如桌面或 `D:\Tools\QuickExit`，不要放在 `Program Files`）。
2. 雙擊執行，允許系統管理員權限（遊戲通常以管理員身分執行，沒有權限就送不出點擊）。
   第一次執行會在同一個資料夾產生 `config.toml`。
3. **校正位置（第一次必做）**
   進入遊戲 → 按 Ctrl 叫出滑鼠 → 把滑鼠移到右上角「退出」按鈕上 → 按 `Shift+F9`。
   聽到「叮」一聲代表成功，程式會記住位置，並自動把快捷鍵綁定到這個遊戲。
4. 之後在副本內按 `F9` 就會自動點擊退出。

> 如果點完退出後會跳出確認視窗：叫出確認視窗後，把滑鼠移到「確認」按鈕上按 `Shift+F10`，
> 之後按 F9 就會連續點擊退出和確認。

## 預設快捷鍵

| 功能 | 預設 |
|---|---|
| 自動點擊退出 | `F9` |
| 校正退出按鈕位置 | `Shift+F9` |
| 校正確認按鈕位置 | `Shift+F10` |

都可以在 `config.toml` 修改，**存檔後自動生效，不用重開程式**。
可用按鍵：`F1`~`F24`、`A`~`Z`、`0`~`9`、`Numpad0`~`9`、`Insert`、`Delete`、`Home`、`End`、
`PageUp`、`PageDown`、`Pause`、`Mouse3`（中鍵）、`Mouse4` / `Mouse5`（滑鼠側鍵），
可搭配 `Ctrl+` / `Alt+` / `Shift+` / `Win+`。

快捷鍵被觸發時會被攔截，不會傳到遊戲；設定了目標程式後，在其他視窗按同樣的鍵完全不受影響。

## 喚出滑鼠的方式（`[cursor]`）

| mode | 行為 | 適用 |
|---|---|---|
| `hold`（預設） | 點擊期間一直按住 Ctrl | 遊戲設定「按住 Ctrl」或「按一下 Ctrl」都適用 |
| `tap` | 先按一下 Ctrl，點完再按一下收起（`hide_after`） | 「按一下 Ctrl」切換模式 |
| `none` | 不按鍵 | 滑鼠本來就看得到時 |

喚出鍵可以用 `key` 改成 `RCtrl`、`LAlt` 等。

> 注意：如果你用的是「按一下切換」模式，而且觸發前滑鼠**已經是叫出來的狀態**，
> `hold` 模式會把滑鼠切回隱藏，導致點不到。這種情況請先把滑鼠收起來再按快捷鍵。

## 點不到 / 沒反應時

- **完全沒反應**：確認主控台視窗顯示「權限：系統管理員」。
- **滑鼠有移過去但沒點到**：把 `[cursor] reveal_delay_ms` 或 `[click] move_delay_ms` 調大（例如 300 / 150）。
- **點的位置偏掉**：重新用 `Shift+F9` 校正。位置以遊戲畫面比例儲存，換解析度通常不用重新校正。
- **換遊戲 / 目標程式綁錯**：把 `target_process` 改回 `""`，再校正一次。

## 注意

本工具是模擬鍵盤與滑鼠輸入。部分線上遊戲的使用條款禁止外部輸入工具，請自行評估風險。

---

## 開發

- 語言：Rust（`x86_64-pc-windows-gnu` 工具鏈，不需要 Visual Studio）
- 編譯 + 打包：`powershell -ExecutionPolicy Bypass -File .\build.ps1` → 輸出 `dist\QuickExit-v<版本>.zip`
- 測試：`cargo test`
- GNU 的 linker 不支援中文路徑，所以 `.cargo/config.toml` 把編譯輸出目錄設在純英文路徑。

### 版本控制流程

1. 修改程式並 commit。
2. 發佈新版時：修改 `Cargo.toml` 的 `version`、在 `CHANGELOG.md` 新增一段。
3. `git commit -am "Release vX.Y.Z"`，`git tag vX.Y.Z`。
4. 執行 `build.ps1` 產生 zip。

| 檔案 | 說明 |
|---|---|
| `src/main.rs` | 啟動流程、鍵盤/滑鼠 Hook、退出與校正動作 |
| `src/config.rs` | 設定檔讀寫（帶註解的 TOML） |
| `src/hotkey.rs` | 快捷鍵字串解析 |
| `src/win.rs` | Win32 輔助：視窗位置、模擬按鍵與點擊、權限 |
