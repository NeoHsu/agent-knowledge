# Portable Agent Memory System — Design Plan

> 可攜式 Agent 記憶系統。記憶跟著人走，不綁定任何平台。

## 設計原則

- 你的知識是你的，平台只是介面
- 確定性操作用程式，LLM 只做判斷
- 一個 git repo 帶著走，任何平台都能用
- 不過度工程 — 先跑起來再迭代

---

## 核心架構

本專案分成兩層：

- `agent-knowledge` repo：CLI、schema、skills、readers、docs、CI/release。這層可以發布，不追蹤私人 `memory.db`。
- runtime/private knowledge store：實際的 `memory.db`、`index/`、可選 profile/config。這層由 `AGENT_KNOWLEDGE_HOME` 或當前 repo root 決定。

```
agent-knowledge/                  (CLI/project repo)
├── skills/
│   └── memory/SKILL.md           唯一的 memory skill
│                                 (CLI 操作 + 復盤 workflow)
│
├── readers/                      session log 讀取器
│   ├── claude-code.sh
│   ├── hermes.sh
│   └── discord.sh
│
├── bin/
│   └── mem                       Rust CLI
│
├── config.yaml                   環境設定
└── schema/
    └── memory-schema.sql         DB schema
```

```
runtime/private store             (local 或 private data repo)
├── memory.db                     SQLite source of truth
├── index/                        Tantivy 搜尋索引（可從 db 重建）
├── profile/                      可選：身份/偏好
└── config.yaml                   可選：環境設定
```

---

## 元件設計

### 1. Profile — 穩定身份（很少變）

```yaml
# profile/identity.yaml
name: Neo Hsu
email: neo_hsu@example.com
role: Security Engineer
expertise:
  - OT security
  - agent architecture
  - DevSecOps
```

```yaml
# profile/preferences.yaml
communication:
  language: zh-TW
  style: concise
  no_emoji: true
  no_trailing_summary: true

coding:
  no_unnecessary_comments: true
  no_premature_abstraction: true
  prefer_edit_over_create: true
```

任何平台的 agent 都能讀這兩個檔案來了解你。

---

### 2. Memory DB — 結構化記憶

```sql
-- schema/memory-schema.sql
-- SQLite 只負責儲存資料，搜尋索引由 Tantivy 處理

CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,                -- user | feedback | project | reference
    name TEXT NOT NULL,
    description TEXT,
    content TEXT,
    tags TEXT,                         -- JSON array: ["person:alice", "domain:security"]
    scope TEXT DEFAULT 'global',       -- global | project:<name>
    source TEXT NOT NULL,              -- manual | agent | daily_retro | weekly_retro
    confidence TEXT DEFAULT 'medium',  -- high(手動) | medium(agent即時) | low(retro推斷)
    protected BOOLEAN DEFAULT FALSE,   -- manual 來源自動標記 protected
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME,               -- project 類建議必填
    valid_until DATETIME,              -- 軟過期：被新記憶取代時標記，不刪除
    superseded_by TEXT,                -- 取代此記憶的新記憶 ID（反向指標）
    version INTEGER DEFAULT 1,
    access_count INTEGER DEFAULT 0,    -- 被查詢到的次數
    last_accessed_at DATETIME          -- 最近一次被查詢的時間
);

-- 歧義/待確認記錄：查詢時發現多種可能的記錄，留給復盤處理
CREATE TABLE ambiguities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    query TEXT NOT NULL,               -- 觸發歧義的查詢
    memory_ids TEXT NOT NULL,          -- JSON array: 涉及的記憶 IDs
    context TEXT,                      -- 當時的情境描述
    resolution TEXT,                   -- resolved | pending
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    resolved_at DATETIME
);

-- 操作歷史：追蹤所有記憶修改，memory.db 是 binary 無法 git diff
CREATE TABLE changelog (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id TEXT NOT NULL,
    action TEXT NOT NULL,              -- save | update | supersede | delete | merge
    old_content TEXT,                  -- 修改前內容（update/supersede 時記錄）
    new_content TEXT,                  -- 修改後內容
    source TEXT,                       -- 操作來源: manual | agent | daily_retro | weekly_retro
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_type ON memories(type);
CREATE INDEX idx_scope ON memories(scope);
CREATE INDEX idx_expires ON memories(expires_at);
CREATE INDEX idx_source ON memories(source);
CREATE INDEX idx_valid_until ON memories(valid_until);
CREATE INDEX idx_access ON memories(access_count);
CREATE INDEX idx_confidence ON memories(confidence);
CREATE INDEX idx_changelog_memory ON changelog(memory_id);
CREATE INDEX idx_changelog_action ON changelog(action);
```

#### 搜尋引擎 — Tantivy

SQLite 儲存資料，Tantivy 負責搜尋索引。兩者分離：

```
寫入: mem save → 寫入 SQLite + 更新 Tantivy 索引
查詢: mem query → Tantivy 搜尋 → 用結果 ID 從 SQLite 取完整資料
重建: mem reindex → 從 SQLite 重建 Tantivy 索引

memory.db (SQLite)          index/ (Tantivy)
├─ source of truth          ├─ 可隨時從 db 重建
├─ local/private data       ├─ .gitignore（不追蹤）
└─ 結構化篩選               └─ 全文搜尋 + BM25 排序
   (type, scope, tags)         (中英文分詞, fuzzy)
```

Tantivy 索引欄位：

```rust
// Tantivy schema
let mut schema_builder = Schema::builder();
schema_builder.add_text_field("id", STRING | STORED);
schema_builder.add_text_field("name", TEXT);           // 分詞索引
schema_builder.add_text_field("description", TEXT);    // 分詞索引
schema_builder.add_text_field("content", TEXT);        // 分詞索引（主搜尋欄位）
schema_builder.add_text_field("tags", TEXT);           // 分詞索引
schema_builder.add_text_field("scope", STRING);        // 精確篩選
schema_builder.add_text_field("type", STRING);         // 精確篩選
schema_builder.add_date_field("created_at", INDEXED);  // 時間排序
```

Tokenizer 設定：

```rust
// CJK 分詞: lindera tokenizer（支援中文、日文、韓文）
// 英文: 標準 tokenizer + stemming
// 混合文本自動處理，不需要手動切換
index.tokenizers().register(
    "multilingual",
    LinderaTokenizer::with_config(/* CJK + Latin */)
);
```

查詢能力：

```
mem query "deploy"                → BM25 全文搜尋
mem query "部署"                  → CJK 分詞後搜尋
mem query "部署 security"         → 中英混合搜尋
mem query "deplpy" --fuzzy        → fuzzy match（容錯）
mem query --type feedback         → Tantivy filter（不走全文）
mem query --scope auto            → Tantivy filter + SQLite scope 偵測
```

#### 記憶類型

| type | 存什麼 | scope |
|------|--------|-------|
| `user` | 身份、角色、專業 | 永遠 global |
| `feedback` | 行為修正/確認 | global 或 per-project |
| `project` | 專案背景、決策、deadline | per-project |
| `reference` | 外部系統指標 | global 或 per-project |

#### Project Scope

```
scope 值：
  global              → 所有專案適用
  project:ot-product  → 只在該專案時載入
  project:discord-bot → 只在該專案時載入

偵測方式：
  git remote get-url origin → 推斷 scope
  例: git@github.com:example/ot-product.git → project:example/ot-product
```

---

### 3. `mem` CLI — Rust 實作

純資料層工具，不含 LLM 邏輯。

#### 指令一覽

```
寫入
────
mem save --type feedback --name no_emoji \
    --tags '["style"]' --scope global \
    --content "不要使用 emoji" \
    --why "使用者明確要求"
    --source agent

mem save --name pr_small \
    --type feedback \
    --scope "project:example/ot-product" \
    --content "PR 拆小逐個 review"

# confidence 自動推斷規則（不需手動指定）：
#   source=manual       → confidence=high
#   source=agent        → confidence=medium
#   source=daily_retro  → confidence=low
#   source=weekly_retro → confidence=low
# 可手動覆蓋: mem save --confidence high ...
#
# 所有寫入/更新/刪除自動記錄到 changelog 表

查詢（Tantivy BM25 搜尋，預設按相關性排序，同分按 created_at DESC）
────
mem query "security review"              # BM25 全文搜尋（中英文）
mem query "部署流程"                      # CJK 分詞搜尋
mem query "deplpy" --fuzzy               # fuzzy match 容錯
mem query --type feedback                # 精確篩選（Tantivy filter）
mem query --tags "deploy"                # 按 tag 篩選
mem query --scope auto                   # global + 當前專案
mem query --expired                      # 已過期記憶
mem query --sort access_count            # 按存取頻率排序
mem query --sort time                    # 按時間排序（新的優先）

# 查詢時自動更新 access_count + last_accessed_at
# 被 valid_until 標記的記憶預設不回傳（加 --include-superseded 顯示）

更新
────
mem update <name> --content "新內容"      # version +1, 記錄 changelog
mem update <name> --add-tags "new_tag"
mem supersede <old_name> <new_name>      # 舊記憶 valid_until=now + superseded_by=new
                                         # 新記憶繼承舊的 tags 和 scope

刪除
────
mem delete <name>                        # 軟刪除：設定 valid_until = now
mem delete <name> --hard                 # 真刪除（protected 需加 --force）

歧義記錄
────
mem ambiguity add --query "PR 策略" \
    --memory-ids '["pr_small", "pr_bundled"]' \
    --context "不確定這個專案要拆小還是合併"
mem ambiguity list                       # 列出待解決的歧義
mem ambiguity list --pending             # 只看未解決的
mem ambiguity resolve <id>               # 標記已解決

歷史
────
mem history <name>                       # 查看某筆記憶的修改歷史（從 changelog）
mem history                              # 最近 20 筆操作
mem history --action delete              # 篩選特定操作類型

維護
────
mem gc                                   # 清理 valid_until 超過 90 天的記憶
mem stats                                # 數量、類型分佈、存取排行、信心度分佈
mem audit                                # 健康檢查：矛盾、孤立、低存取、低信心
mem audit --fix                          # 自動修復可安全處理的問題：
                                         #   • 清理過期記憶
                                         #   • 修復 superseded_by 斷鏈
                                         #   • 重建 Tantivy 索引
mem reindex                              # 從 SQLite 重建 Tantivy 索引
mem export --format markdown             # 給平台載入
mem export --format json                 # 給腳本/agent 用

匯入
────
mem import <file.json>                   # 批次匯入記憶（JSON 格式）
mem import <file.md> --type reference    # 從 markdown 匯入單筆
                                         # 輸出單一 import_complete summary JSON

合併（跨平台衝突處理）
────
mem merge <theirs.db>                    # 合併另一個平台的 memory.db
                                         # 流程：
                                         #   1. 讀取 theirs.db 所有記憶
                                         #   2. strip incoming secrets
                                         #   3. 逐筆和本地比對
                                         #   4. 相同 → 跳過
                                         #   5. 新的 → 匯入
                                         #   6. 衝突 → 記錄到 ambiguities，含 incoming snapshot

隱私
────
mem save 時自動 strip:
  • API keys (pattern: sk-*, ghp_*, xoxb-*, AKIA*)
  • Bearer tokens
  • 密碼欄位 (password=*, secret=*)
  • .env 檔案內容

環境
────
mem context --detect                     # 從 git remote 推斷當前 scope
```

#### 智慧去重流程

```
mem save --name "X" --content "新內容"
    │
    ▼
 (1) 精確匹配：同 name 存在嗎？（SQLite 查詢）
    │
    ├─ 不存在 → (2) 模糊匹配：Tantivy BM25 搜相似內容
    │              │
    │              ├─ 沒有相似（score < 閾值）→ 直接存入
    │              │
    │              └─ 找到相似 → 回傳候選給呼叫者
    │
    └─ 存在 → 回傳現有記憶給呼叫者
```

回傳格式（JSON）：

```json
// 沒有重複
{"status": "saved", "id": "no_emoji", "version": 1}

// 精確匹配
{
  "status": "duplicate_found",
  "match_type": "exact_name",
  "existing": {
    "id": "no_emoji",
    "content": "不要使用 emoji",
    "version": 2
  },
  "new_content": "不要在回覆中使用 emoji"
}

// 模糊匹配
{
  "status": "similar_found",
  "match_type": "fts5",
  "candidates": [
    {"id": "emoji_policy", "content": "...", "score": 0.82}
  ],
  "new_content": "..."
}
```

呼叫者（Agent / 人 / 復盤）自行判斷要 update、merge 或 skip。
`mem` 不做語意判斷 — 那是 LLM 的事。

#### 衝突處理

信任層級（高 → 低）：

```
(1) manual       你手動存的（自動標記 protected）
(2) agent        你即時觸發 Agent 存的
(3) daily_retro  日復盤產生的
(4) weekly_retro 週復盤產生的
```

規則：

```
┌─────────────────────┬────────────────────────────┐
│ 情況                 │ 處理                        │
├─────────────────────┼────────────────────────────┤
│ 同名 + 內容相同      │ 跳過，回報 identical         │
│ 同名 + 新的更詳細    │ 回傳候選，呼叫者決定         │
│ 同名 + 內容矛盾      │ 比較 source 優先級           │
│   高 >= 低           │   → 回傳 conflict，問呼叫者  │
│   低 → 高            │   → 拒絕，回報 rejected      │
│ 刪除 protected       │ 拒絕（除非 --force）         │
│ 同時寫入             │ flock + version check       │
└─────────────────────┴────────────────────────────┘
```

並發控制：

```
Agent A: mem update user_neo ...
    │
    ▼
flock memory.db → 取得鎖 → 寫入 → 釋放
                                    │
Agent B: mem update user_neo ...    │
    │                               │
    ▼                               ▼
flock → 等待 → 取得鎖 → version 已變
    │
    ▼
回傳 version_conflict，要求重讀後重試
```

---

### 4. Memory Skill — 唯一的 Skill

```
skills/
└── memory/
    ├── SKILL.md                   # < 200 行概覽
    └── references/
        ├── cli-guide.md           # mem CLI 完整用法
        ├── tag-rules.md           # tag 提取規則和範例
        ├── daily-retro.md         # 日復盤完整步驟
        └── weekly-retro.md        # 週復盤完整步驟
```

SKILL.md（< 200 行）為概覽 + 觸發規則，細節用 progressive disclosure 載入 references/：

```
SKILL.md 結構：
├── When to Use — 觸發條件
├── Quick Reference — mem CLI 摘要
│   └── 詳見 references/cli-guide.md
├── Tag Extraction — 提取規則摘要
│   └── 格式: type:value（例: person:alice, domain:security）
│   └── 詳見 references/tag-rules.md
├── Daily Retrospective — 步驟摘要
│   └── 詳見 references/daily-retro.md
└── Weekly Retrospective — 步驟摘要
    └── 詳見 references/weekly-retro.md
```

載入層級：
```
Level 0: skill 名稱 + 描述        (~100 tokens)  — 啟動時
Level 1: SKILL.md 概覽             (~150 行)      — 觸發時
Level 2: references/xxx.md         (按需)         — 執行時
```

觸發方式：

```
/memory save ...          → Agent 讀 Part 1
/memory retro daily       → Agent 讀 Part 2
/memory retro weekly      → Agent 讀 Part 2
「幫我存這個」             → Agent 載入 SKILL.md，走 Part 1
「做今天的復盤」           → Agent 載入 SKILL.md，走 Part 2
```

---

### 5. 復盤 Workflow

#### 三種寫入路徑

```
 (1) 即時觸發            (2) 日復盤            (3) 週復盤
 你主動說「存這個」       每天手動觸發          每週手動觸發

 對話中                  讀 session logs       讀本週 changelog
 ──────                  ──────────           + memory.db
 Agent 立即              提取漏存的            + ambiguities
 mem save                新知識               ──────────
     │                       │               合併 / 升級 /
     │                       │               信心度校準 / 清理
     │                       │                    │
     └───────────┬───────────┴────────────────────┘
                 │
                 ▼
         memory.db（local/private data）
         （唯一的知識終點，沒有中間產物）
                 │
                 ▼
            local/private data repo 可選 git commit + push
```

#### 日復盤流程

```
觸發: /memory retro daily（或「做今天的復盤」）
    │
    ▼
Agent 載入 SKILL.md 復盤段落
    │
    ▼
(1) 偵測平台 → 呼叫對應 reader 取得今日 session logs
    │
    ▼
(2) mem export --format json → 取得現有記憶
    │
    ▼
(3) Agent（LLM）分析 logs vs 現有記憶：
    │
    ├─ 新知識提取
    │   • 什麼是新學到的？→ mem save（順便提取 tags）
    │   • 什麼需要更新？→ mem update / mem supersede
    │   • 什麼已過期？→ mem delete（protected 的只建議）
    │   • 什麼重複了？→ 合併
    │
    ├─ 矛盾偵測
    │   • 掃描今日新增/修改的記憶 vs 現有記憶
    │   • 發現語意矛盾 → mem ambiguity add
    │   • 例：「PR 拆小」vs「PR 合併」
    │         → 可能是不同專案的不同偏好
    │         → 記錄歧義，下次查詢時提醒
    │
    └─ 歧義處理
        • mem ambiguity list --pending
        • 嘗試解決：補上 scope 區分？刪除過期的那筆？
        • 無法解決的保留給週復盤或問使用者
    │
    ▼
(4) 回報結果：
    「3 筆新增、1 筆更新、1 筆取代。
     ⚠️ 1 筆矛盾待確認：pr_small vs pr_bundled
     建議確認: user_pnpm 是否還有效？」
    │
    ▼
(5) 若 runtime store 是 private data repo，git commit + push
```

#### 週復盤流程

```
觸發: /memory retro weekly
    │
    ▼
(1) 讀取本週的 changelog（日復盤已處理過的結果）
    + 完整 memory.db + ambiguities 表
    │
    ▼
    ※ 不重讀 raw session logs — 日復盤已經提取過了（增量處理）
    ※ 週復盤關注的是「記憶的品質」而非「對話的內容」
    │
    ▼
(2) Agent 做高層次整理：
    │
    ├─ 知識升級
    │   • 從 changelog 看本週新增了什麼 → 有 pattern 嗎？
    │   • 重複出現 3+ 次的 pattern → 該變成 skill？
    │   • 相似記憶合併
    │   • profile 需要更新嗎？
    │
    ├─ 信心度校準
    │   • confidence=low 的記憶 → 還正確嗎？
    │   • 被多次存取的 low confidence → 升級為 medium
    │   • 從未被存取的 medium → 降級或標記建議清理
    │
    ├─ 清理
    │   • 過期的 project 記憶清理
    │   • access_count = 0 且超過 30 天 → 標記建議清理
    │   • 跨專案的關聯發現
    │   • changelog 堆積清理（保留 90 天）
    │
    ├─ 矛盾總清
    │   • 處理本週所有 pending ambiguities
    │   • 嘗試用本週 changelog 脈絡解決
    │   • 無法自動解決的 → 列出問使用者
    │
    └─ 健康檢查（mem audit 的子集）
        • 記憶總數 / 類型分佈 / scope 分佈 / 信心度分佈
        • 低存取記憶佔比
        • valid_until 記憶堆積量
        • superseded_by 鏈完整性
    │
    ▼
(3) 執行修改 + 回報
    │
    ▼
(4) 若 runtime store 是 private data repo，git commit + push
```

---

### 6. Session Log Readers

每個平台一個確定性腳本，輸出統一 markdown 格式：

```
readers/
├── claude-code.sh      # 讀 ~/.claude/projects/*/sessions/
├── hermes.sh           # 讀 ~/.hermes/sessions/
└── discord.sh          # 透過 API 讀訊息歷史
```

首次設定時 LLM 探測環境，寫入 config.yaml：

```yaml
# config.yaml
platforms:
  claude-code:
    detected: true
    session_log_path: ~/.claude/projects/-home-node/sessions/
    log_format: jsonl
    reader: readers/claude-code.sh
  openab-discord:
    detected: true
    channel_id: "1428761906880970915"
    reader: readers/discord.sh
  hermes:
    detected: false
```

之後復盤時直接讀 config，不再探測。

```
分工：

  reader（程式）  → 讀取 + 格式轉換（確定性）
  Agent（LLM）    → 判斷什麼值得存（它擅長的）
```

---

### 7. 跨平台使用

不需要 adapter。任何能跑 shell 的平台，在其設定檔中加一段指向 mem CLI 即可：

```
Claude Code:  CLAUDE.md 加一段 → 指示用 mem CLI
Cursor:       .cursorrules 加一段
Hermes:       config 指向 skills/memory/SKILL.md
```

知識庫同步靠 runtime/private data store。若你選擇用 private Git repo 保存 runtime store，才需要 git：

```
# 新環境
git clone git@github.com:you/private-memory-store.git ~/.agent-knowledge-data
export AGENT_KNOWLEDGE_HOME=~/.agent-knowledge-data

# 日常
cd ~/.agent-knowledge-data && git pull  # 取得其他平台存的記憶
# ... 工作 ...
git add -A && git commit && git push  # 推送新記憶
```

---

## 查詢流程

```
Agent 開始工作
    │
    ▼
mem context --detect
→ scope = "project:example/ot-product"
    │
    ▼
mem query --scope auto --type feedback
→ 回傳: global 記憶 + project:example/ot-product 記憶
    │
    ▼
Agent 帶著正確的 context 開始工作
```

`--scope auto` = global + 當前偵測到的 project scope。

---

## 完整資料流

```
╔═══════════════════════════════════════════════════════════╗
║                    你的日常工作                            ║
╠═══════════════════════════════════════════════════════════╣
║                                                           ║
║  Session 1         Session 2         Session 3            ║
║  (Claude Code)     (Hermes)          (Cursor)             ║
║  ┌──────────┐     ┌──────────┐     ┌──────────┐          ║
║  │ 對話     │     │ 對話     │     │ 對話     │          ║
║  │          │     │          │     │          │          ║
║  │ 「存這個」│     │          │     │          │          ║
║  └────┬─────┘     └──────────┘     └──────────┘          ║
║       │                                                   ║
║       ▼ 即時觸發                                          ║
║  mem save → memory.db                                     ║
║                                                           ║
╠═══════════════════════════════════════════════════════════╣
║                                                           ║
║  /memory retro daily                                      ║
║       │                                                   ║
║       ▼                                                   ║
║  readers 取得今日所有 session logs                          ║
║       │                                                   ║
║       ▼                                                   ║
║  Agent 分析 → mem save/update/delete → memory.db          ║
║       │                                                   ║
║       ▼                                                   ║
║  private data repo 可選 git commit + push                 ║
║                                                           ║
╠═══════════════════════════════════════════════════════════╣
║                                                           ║
║  /memory retro weekly                                     ║
║       │                                                   ║
║       ▼                                                   ║
║  本週 changelog + memory.db + ambiguities                  ║
║  （不重讀 raw logs — 日復盤已處理過）                        ║
║       │                                                   ║
║       ▼                                                   ║
║  Agent 高層整理:                                           ║
║    • pattern → skill 升級                                 ║
║    • 信心度校準 / 合併 / 清理 / profile 更新                ║
║       │                                                   ║
║       ▼                                                   ║
║  memory.db + 可選 profile/skill 建議                       ║
║       │                                                   ║
║       ▼                                                   ║
║  private data repo 可選 git commit + push                 ║
║       │                                                   ║
║       ▼                                                   ║
║  所有平台下次同步 runtime store 時取得更新                  ║
║                                                           ║
╚═══════════════════════════════════════════════════════════╝
```

---

## 實作順序

### Phase 1 — 骨架（先能跑）

1. 建立 GitHub private repo `agent-knowledge`
2. 建立目錄結構 + config.yaml + schema（含 changelog 表）
3. 實作 `mem` CLI 基本功能（Rust）
   - `mem save` / `mem query` / `mem update` / `mem delete`
   - SQLite（資料儲存）+ Tantivy（搜尋索引）
   - Tantivy lindera tokenizer（CJK 中英文分詞）
   - scope 支援
   - confidence 自動推斷（source→confidence mapping）
   - JSON 輸出
   - BM25 排序，同分按 created_at DESC
   - 查詢時自動更新 access_count + last_accessed_at
   - 所有寫入/修改自動記錄 changelog
   - `mem reindex`（從 SQLite 重建 Tantivy 索引）
4. 寫 skills/memory/SKILL.md + references/（progressive disclosure）
5. profile/ 初始化

### Phase 2 — 去重、衝突與生命週期

6. `mem save` 的智慧去重（Tantivy BM25 比對 + 回傳候選）
7. `mem save` 自動 strip secrets（正則過濾）
8. source + protected 機制
9. `mem supersede` — 軟取代（valid_until + superseded_by）
10. flock 並發控制
11. `mem gc` / `mem stats` / `mem audit` / `mem audit --fix`
12. `mem history`（從 changelog 查看記憶修改歷史）
13. `mem query --fuzzy`（Tantivy fuzzy search）

### Phase 3 — 復盤與歧義

14. 寫第一個 reader（claude-code.sh）
15. ambiguities 表 + `mem ambiguity` 子指令
16. `mem import`（JSON 批次、markdown/文字單筆）
17. references/daily-retro.md + weekly-retro.md
18. 測試日復盤流程（含矛盾偵測 + 歧義處理）
19. 測試週復盤流程（含信心度校準 + 矛盾總清 + 健康檢查）

### Phase 4 — 擴展

20. 更多 readers（hermes, discord）
21. `mem export` 各種格式
22. `mem merge`（跨平台 memory.db 合併 + 衝突記錄）
23. 在其他平台驗證可攜性
24. 可選：embedding 介面預留（`mem query --semantic`，不實作後端）

---

## 不做的事（有意識的決定）

| 不做 | 原因 |
|------|------|
| Adapter 同步腳本 | 各平台能跑 CLI 就夠，不需要格式轉換 |
| 復盤結果存檔 | 復盤是過程不是產物，結果直接進知識庫 |
| 自己存 raw log | 用各平台自己的 session log |
| LLM 呼叫層 | Agent 本身就是 LLM，不需要額外 API 串接 |
| 雙向平台同步 | 單向 git pull/push，避免衝突 |
| 向量搜尋 / embedding | Tantivy BM25 + CJK 夠用，Phase 4 預留介面 |
| 硬刪除（預設） | 用 valid_until 軟取代，保留歷史 |
| 知識圖譜 | 用 tags 做輕量關聯，不建完整圖譜 |
| ADD-only 模式 | 有 update 需求，但用 supersede 保留舊版本 |

---

## 借鑒來源

| 改進 | 來自 | 實作方式 |
|------|------|----------|
| access_count + last_accessed_at | agentmemory | DB 欄位，query 時自動更新 |
| 自動 strip secrets | agentmemory | mem save 時正則過濾 |
| valid_until 軟取代 | gbrain | mem supersede + mem delete 預設軟刪 |
| superseded_by 反向指標 | gbrain | 取代鏈追蹤，audit 檢查完整性 |
| 矛盾偵測 | gbrain | 日/週復盤流程中 Agent 掃描 |
| 零 LLM 關聯 (tags) | gbrain | Agent 寫入時提取 typed tags |
| 新記憶優先排序 | mem0 | query 結果按 created_at DESC |
| confidence 分級 | graphify | source→confidence 自動推斷 + 週復盤校準 |
| changelog 追蹤 | graphify | 所有修改記錄，支援 mem history + 週復盤增量 |
| 歧義記錄 | 原創 | ambiguities 表，留給復盤處理 |
| 健康檢查 | gbrain | mem audit / audit --fix + 週復盤 |
