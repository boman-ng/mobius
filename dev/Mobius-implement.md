# Mobius Rust 单二进制工程实现蓝图

## 1. 总装目标、边界与依赖法则

本蓝图把 `dev/mobius-model.md` 与 `dev/Mobius-subagent.md` 落成两个彼此无知、可独立实现和验证的模块，
并规定 main Agent 如何在两者之间完成一次性转义。它描述目标架构，不承担任何现有实现的兼容、迁移或
保留责任。

系统分为三个 owner：

| Owner | 唯一职责 |
|---|---|
| Model Core | 理论对象、transition input、identity、guard、reducer、Trail、projection、Evidence admission、artifact store 与 application service |
| Subagent Skill | 委托角色、basic/result envelope、Judge material freeze、effect 声明与原生 Runtime 生命周期 |
| Main Agent Composition | 是否委托、delegation baseline、Runtime 结果核查、effect 处理、候选转义、正式 Judgment、人类确认协调与 Core 提交 |

唯一允许的依赖方向是：

```text
            Main Agent Composition
                 /             \
                v               v
          Model Core       Subagent Skill
```

因此：

- Model Core 不认识 subagent、role、thread、result envelope、Judge disposition 或 Runtime 状态；
- Subagent Skill 不认识 Objective、Map、Stage、Attempt、Evidence、Decision、Trail、数据库或 Core API；
- 两者不共享 schema、identity、freeze codec、生命周期或持久化；
- main Agent 可以完全不使用 Subagent Skill 而运行 Mobius；
- subagent 输出只能成为 main Agent 的候选 observation、effect、artifact、advice 或 provenance；
- 正式 `ReviewDecision` 与 `J_b` 只能由 main Agent 构造，并由 Core guard 接纳或拒绝。

工程目标是：

1. 在每个 project root 的 `.mobius/` 私有缓存目录内，用唯一 SQLite 数据库持久化该 project 的 Trail；
2. 用一个领域内核实现全部 Programmatic guard、reducer 与 replay；
3. 用 Core-owned artifact store 实现可审计的 Evidence freeze；
4. 用一个 Rust 单二进制运行时承载 Core service、最小 MCP、无业务状态 mutation authority 的 CLI、hook
   handler 与维护逻辑；
5. 用独立薄 skill 提供可选的 subagent 委托能力；
6. 用 session/run CSV snapshot 提供人类可浏览的派生 read model，并用一份窄的 `interaction.md` 为 main
   Agent 的 Route 设计保留非权威背景；
7. 保持一个事实源、一个 mutation service、一条完成路径。

这里的“单二进制”指每个受支持 target 只有一个名为 `mobius` 的可执行运行时：安装后不需要 Python
解释器、virtualenv、语言包管理器、sidecar、第二个 helper executable 或常驻全局 daemon。Host 仍需直接读取
plugin manifest、MCP/hook 配置、`SKILL.md` 与 references；SQLite、artifact 和其他 project-local state 仍是数据，
不属于可执行文件。

不实现 hosted service、遥测、分布式共识、全局 daemon、第二套 Agent runtime、共享桥接协议、并行的业务状态
引擎或 Python fallback。

## 2. Model Core：对象、状态与转移覆盖

### 2.1 Rust crate 与领域模块

Core、transport 与本地运维入口组成一个 Cargo package，只产生一个 binary target：

```text
plugins/mobius/
  runtime/
    Cargo.toml
    Cargo.lock
    src/
      main.rs                 # 只做 mode dispatch、I/O wiring 与 exit status
      domain/
        mod.rs
        types.rs              # 十一类一等对象、状态、identity 与 transition input
        guards.rs             # 纯 Programmatic guards
        reducer.rs            # δ、派生查询与 replay
      application/
        mod.rs
        service.rs            # 唯一 application service
        admission.rs          # live host admission boundary
      infrastructure/
        mod.rs
        artifacts.rs          # CoreSnapshot capture、read、integrity 与 GC
        sqlite.rs             # schema、transaction 与 projection rebuild
      presentation/
        mod.rs
        report.rs             # context-dark session/run CSV renderer
      transport/
        mod.rs
        mcp.rs                # stdio MCP adapter
        cli.rs                # audit/doctor/report adapter
        hooks.rs              # pre-tool-use 与 stop adapter
      error.rs                # 稳定的结构化错误类型
```

`domain` 不得依赖 application、infrastructure、presentation、transport、skills、hooks、Codex Runtime 类型或 Subagent
references。Application 依赖 domain，并拥有以 domain 类型表达的 ports；infrastructure、presentation 与
transport 实现外层 adapter，依赖方向不能反向进入领域模块。所有 module 最终编译进同一个 `mobius`
executable，不产生可动态
加载的 Core library、第二个 binary target 或脚本入口。

Rust store 必须嵌入或随 `mobius` 链接 SQLite；运行 Core 不通过 CLI。插件不捆绑或下载 SQLite CLI，但
Agent 直接读取状态的受支持 host 必须另有 canonical absolute `sqlite3 >= 3.40.1`。第三方 crate 必须通过
提交的 `Cargo.lock` 锁定，并经过 license、supply-chain 与 platform build 审查；单二进制目标不构成引入
框架或后台服务的理由。

### 2.2 Typed mapping

实现前必须建立一份机械可检查的持久化映射，覆盖：

- 十一类一等对象及其全部字段；
- `ObjectiveState`、`NavState`、Route 与 Attempt lifecycle；
- 模型第 10 节全部 transition input；
- 每类对象的理论 identity 与结构相等；
- set-like collection 的规范表示；
- event schema 与确定性解析规则。

对象进入 Trail 后不可原地修改。Revision、新 Attempt、新 Decision 与新 transition 必须产生新 identity 或新
事实。数据库 row id、文件路径、Runtime id 与时间戳都不能充当理论 identity。

### 2.3 纯 reducer

```text
reduce(q, valid_transition_input) -> q'
replay(trail_prefix) -> q
```

Reducer 只读取前态 `q` 与已经通过 live admission 的 typed transition input。它不读取时钟、文件、网络、
环境变量、host、thread、role、result 或 projection。相同输入必须产生相同后态。

### 2.4 Guard coverage

实现必须逐项覆盖模型第 10 节：

| Transition family | 核心 Programmatic 条件 |
|---|---|
| Activate | 有效 `H^{confirm}`、Criteria 非空、`FreshBatch`、project 无其他 active Objective |
| InstallMap | 非空 Stage、Objective Criteria 包含关系、每 Stage 非空 Criterion、DAG、`π` 全函数与稳定次序、owner 全函数、`Contract_μ` 全域且逐 Stage 包含 outcome、owned Criteria、相关 Objective boundaries 与 output、Cover 绑定、final-integration 唯一性与 cross-stage/全依赖覆盖、carry 全域与依赖闭包、`TreeCompatible`、Route Structural Context |
| Route | current Stage、Structural Context、available status、Fresh |
| Attempt | ordinal、Acceptance Context、running/sealed/closed lifecycle |
| Evidence | 完整 `EvidenceAdmission_q`，包括 freeze、current subject、purpose、Context 与 claims domain |
| Seal | termination、当前 Attempt 至少一条 Evidence、精确 `U_q` 与 `DependencyView`、Packet Fresh |
| Decision | `Applicable_q`、Criterion domain 完整、action 唯一、accept 全 satisfied；只有 `replace` 拒绝当前 Route |
| CheckWait | `E_b ≠ ∅`、`FreshBatch(E_b)`、逐项 `EvidenceAdmission_q`、精确 `W_q(b) ∪ E_b`、direction 唯一；只有 `new_route` 拒绝当前 Route |
| Remap | current navigation close、proof invalidation、carry 仅在新 Map install 时生效 |
| Revise / Abandon | 当前 active、Objective identity、有效人类确认 |

完成只能由模型的 `Complete ⇔ AllCurrent` 在 `InstallMap` 或 `accept` transition 中派生。Core 不提供
`mark-complete`、`force-pass`、人工 projection patch 或独立 Exit Review 状态。

### 2.5 Invariants

增量 guard 与完整 audit 必须共同覆盖模型 `I1..I19`。尤其要机械证明：

- 每个 Objective 至多一个 current Stage 与 current Attempt；
- Evidence 只在首次进入 Trail 的接纳前态满足 `EvidenceAdmission_q`；
- Packet 精确冻结当前 Context 的完整 Evidence 宇宙；
- `routeStatus=rejected` 只从已经应用的 `Decision(D:replace)` 或 `CheckWait(...,J_b)` 且 `J_b.direction=new_route` 事实派生；
- main Agent 之外的模型外输出不能直接成为 Evidence、Decision 或 transition；
- terminal state 拒绝后续业务转移；
- replay 结果与当前投影相等。

## 3. Trail、SQLite、project binding 与 single-active

### 3.1 Project-scoped private cache layout

```text
<project-root>/.mobius/
  mobius.sqlite3
  artifacts/
    blobs/
    staging/
  views/                   # 可删除的人类视图与 Route 设计交互摘要
  .gitignore
```

核心存储不变量是：

```text
∀ project p: |MobiusSQLite(p)| = 1
MobiusSQLite(p) = { <canonical-project-root>/.mobius/mobius.sqlite3 }
```

每个 canonical project root 恰好维护一份 Mobius 私有缓存目录和一份 `mobius.sqlite3`。这是 Mobius 在该
project 中创建、识别和维护的唯一 SQLite 数据库，容纳全部 Objective streams、Trail、projection 与 binding；
不得按 Objective、agent、thread、transport、功能或 invocation 拆分数据库。SQLite 自身可能生成的
`mobius.sqlite3-wal` 与 `mobius.sqlite3-shm` 是同一数据库的事务伴随文件，不是第二份数据库。

该不变量不授权 Mobius 扫描、删除或接管 project 中由其他应用拥有的 SQLite 数据库；它只约束 Mobius 自己
的存储边界。

数据库路径只能从当前 canonical project root 推导；不得回退到 home、XDG cache、系统临时目录、workspace
外路径或多个 project 共用的全局数据库。

“私有缓存”表示该目录由 Mobius 独占管理、对其他模块不透明，并应整体排除在 Git 之外；它不表示全部内容
都可丢弃。`trail_events`、binding 与已被 Trail 引用的 artifact 是持久事实，删除会造成真实数据丢失；只有
projection、staging、`views/` 和明确判定为 unreachable 的 blob 才能按本蓝图的 rebuild 或显式清理规则处理。只有
Core-owned service、artifact adapter 与 presentation renderer 可以写各自区域；main Agent 只能执行第 4.7 节定义的
canonical SQLite observation、formal Review 对 exact `core_snapshot` 所需的 digest/size 校验和必要 byte-range
读取，以及第 3.6、9.1 节定义的当前 Objective `interaction.md` 窄只读。除此之外，普通文件工具、Skill 与 main
Agent 都不得解析或直接修改该目录；subagent 不得以任何方式访问 `.mobius/`。Private 也不等同于加密或秘密存储，
敏感材料仍须遵守 host 的数据边界。

首次初始化由 host 从 allowed workspace roots 中选择一个 root，并建立 binding：

1. 对 root 执行 exact `realpath`；
2. 拒绝 path traversal、跨 workspace root 与非 project root；
3. 拒绝 `.mobius/`、数据库、artifact、staging 与 views managed root 为 symlink；
4. 生成 `project_id`，保存 canonical-root digest；
5. 返回 `project_id`。

首次初始化使用独立的 bootstrap protocol，而不套用尚未存在的 project binding：

1. 安全创建或验证 canonical root 下唯一的 `.mobius/` 目录；并发创建者只能得到同一个非 symlink 目录；
2. 安全打开该目录内唯一的 `mobius.sqlite3`，并以 `BEGIN EXCLUSIVE` 串行化 bootstrap；发现第二个候选数据库
   或指向 project scope 外部的数据库路径时拒绝；
3. 在 transaction 中读取 binding；若已存在且 root digest 相同，直接返回既有 `project_id`；若不同则拒绝；
4. 若 binding 不存在，则原子创建 schema、单一 `project_id`、root digest 与 bootstrap request metadata；
5. commit 后幂等创建或验证 artifact、views 目录与内容为 `*` 的 `.gitignore`；该 policy 连自身也排除，避免
   repeated default `git clean` 先剥离 ignore policy；缺少的空目录可补齐，未知文件不得删除；
6. 响应丢失后的重试重新读取既有 binding，不能生成第二个 `project_id`。

两个并发初始化最多有一个创建数据库与 binding，另一个等待后返回同一个结果。Crash before commit 由 SQLite 回滚；
crash after commit 由重试补齐非业务目录。任何 half-created database、错误 schema、managed-path symlink 或
不同 root binding 都显式报错，不通过删除目录重试来掩盖。未知的非 owned symlink 不被跟随、删除或当作
Mobius database candidate；它也不能替代上述 managed path 的 symlink 校验。并发初始化期间，目录枚举后、
候选打开前消失的临时文件按“不再存在”处理；除此之外的候选打开或读取错误继续 fail closed。

首次初始化是尚无 `project_id` 时的唯一 API 例外。此后每个 Core API 都必须提交
`project_root + project_id`，并在读取或写入前重新校验 canonical root、containment 与 binding。项目移动与
rebind 不属于本蓝图；路径 mismatch 必须 fail closed，不能静默建立新 identity。

### 3.2 Trail 是唯一业务事实源

`trail_events` 保存模型 transition input。ObjectiveState、Ω、Route status、Attempt lifecycle、proof、
KnownRoutes 与 Manifest 都从 Trail 派生。

SQLite 采用 append-log + rebuildable read model：`trail_events` 的正常路径只有 `INSERT`，已提交事实不做
`UPDATE` 或 `DELETE`；`schema_meta` 与 `objective_streams` 只保存日志 head，projection 表是可替换、可删除、
可从 Trail 重建的索引缓存。Maintenance 可以重建 projection 或清理不可达 artifact，但不能改写 Trail。
这是一种日志型数据库设计，不是通用 workflow engine，也不引入第二份 ledger。

Event envelope 可以保存以下 reducer-inert 运维元数据：

- project-scoped `request_id` 与 request payload hash；
- received time；
- human-gated transition 可选的 opaque host/UI confirmation audit reference。

Agent/thread/role/result/usage 不进入 Trail。审计元数据不进入对象 identity、不改变 reducer 结果，也不能成为
另一份业务事实源。Evidence provenance 可以包含由 main Agent 选择的普通 source、command 或外部对象
locator，但不能保存或依赖 Subagent schema。

### 3.3 最小存储职责

SQLite 至少表达：

| Store | 职责 |
|---|---|
| `schema_meta` | schema、project binding 与 project-global head |
| `objective_streams` | Objective stream identity 与 per-stream head |
| `trail_events` | 不可变 transition facts、project order 与幂等 request metadata |
| `objective_projection` | 可重建状态、lifecycle、Manifest 与派生 `is_active` |
| `object_projection` | 可重建 Ω typed lookup 与 accepted event reference |

Projection 只能由 reducer 在 append transaction 内更新，并可全部删除后从 Trail 重建。API、skill、hook、
main Agent 与 subagent 都不能直接写 projection。

main Agent 直接使用 SQLite 的只读 transaction 查询这五张表。普通恢复先读 schema/project head、目标
Objective head 和少量 projection 字段；对象与历史按当前问题使用显式列、谓词、排序与有限 `LIMIT`。正式
Review 读取冻结的精确 identity closure；正式 Wait 在单个 snapshot 中返回机械 count、
payload bytes 与 Context-budget admission，并且只在整个集合被接纳时返回全部匹配 Evidence。
超预算只返回 summary；任何截断、缺行或 count mismatch 都不能推进状态。

读路径不经过 application service、MCP 或自定义 ORM，不提供别名、cursor、chunk DTO、兼容 facade 或第二
查询引擎。Projection 是可重建 read model；它可用于观察，但不能授权写入。写入 admission、显式 audit、
projection rebuild 与 human report 仍从 Trail replay 并比较 projection，负责完整事实等价证明。

### 3.4 每个 project 一个 active Objective

这是 project-scope 的工程不变量，不是临时发布限制：

- 一个 project 同时至多一个 Objective 处于 `Mapping` 或 `Navigating`；
- 历史 `Achieved` 与 `Abandoned` streams 可以共存；
- 新 `ActivateObjective` 必须在 project-global write transaction 中检查全部 streams；
- 两个不同 Objective 的并发 Activate 必须最多成功一个；
- terminal 后可以激活新的 Objective。

Projection 应提供可索引的派生 `is_active` 并使用数据库约束作为第二道机械防线；Trail 仍是该值的唯一来源。

### 3.5 原子 transaction

新请求的固定顺序是：

```text
project binding
→ project-scoped request_id lookup
→ same request/same payload idempotent return，或 same request/different payload reject
→ expected_project_seq + expected_objective_seq check
→ project-global 与 transition guards
→ append immutable event
→ reduce + projection update
→ affected invariants
→ commit
```

`expected_project_seq` 是 project-global Trail head；`expected_objective_seq` 是目标 Objective stream head，
首次 Activate 时为 `0`。Agent 在同一只读 snapshot 中读取两者。所有 mutation 使用同一个 project-global writer critical
section 与 SQLite write transaction。失败时业务状态不变。响应丢失后的同 payload retry 返回既有提交结果；
任一 stale head 都返回 conflict，调用方必须重新读取并重新判断，不自动重放旧意图。

### 3.6 Session/run 人类视图

旧版 session/run 分层只作为人类可浏览体验的参考；本次 refactor 不继承旧路径、CSV schema、可写 ledger 或
兼容合同。新版在唯一 SQLite 数据库之外提供单向 presentation files：CSV read model 可重建，
`interaction.md` 可删除但不承担恢复职责：

```text
<project-root>/.mobius/views/
  codex-session-<session-ref>/
    runs/
      run-<slug>--<objective-id-short>/
        current.csv
        generations/
          generation-<generation-id>/
            meta.csv
            overview.csv
            stage-view.csv
            criterion-view.csv
            route-view.csv
            attempt-view.csv
            evidence-view.csv
            review-view.csv
            timeline.csv
    interactions/
      <slug>--<objective-id-short>/
        revision-<objective-spec-revision>/
          interaction.md
```

这里的 `Run` 只是一个 Objective 的人类呈现单元，不是一等模型对象，也不是 Attempt 的别名。正式 identity
始终是 Objective identity；slug 只改善可读性，短 identity 防止重名。同一 Objective 跨 Codex session 继续时
可以在多个 session view 中出现，但这些副本不改变 Objective、Trail 或完成责任。业务明细表使用 `*-view`
后缀，避免与旧版可写 ledger 或数据库 projection 混淆。

`interaction.md` 是 Copilot 在已接受 activation/revision 后整理的摘要，不是完整 transcript。它与 `runs/`
并列，避免被识别为不完整 CSV run；同一 session/slug/revision 的幂等写覆盖同一文件，不同 revision 使用互不
覆盖的目录，因此延迟的旧写不能降级新 revision 摘要。文件固定包含完整 Objective identity、ObjectiveSpec
revision、action、Interpreted Intent、Confirmed Boundaries、Verified Facts、Challenges and Resolutions 与
Route Notes。它不得包含模型私有思维链、系统提示、secret、scratch reasoning 或未经筛选的工具输出。

`session-ref` 由 host-side presentation adapter 作为经过验证的 path-safe opaque reference 提供。Session ref、
slug、short Objective identity 与 generation id 都经过 path-component encoding 与 containment check 后才能落盘。它不进入 typed
transition input、Trail、reducer、guard、object identity 或 Subagent task；presentation renderer 不解析其 Runtime
语义。缺少 native session reference 只使自动 session view 不可用，不得阻断业务 transition，也不得由 main
Agent 在 prompt 中维护一个替代 session id。

CSV 面向人而不是状态重建：

- `overview.csv` 给出 Objective、当前状态、revision、Map 与固定 heads；
- `stage-view.csv`、`criterion-view.csv` 与 `route-view.csv` 给出当前和历史结构；
- `attempt-view.csv` 给出 ordinal、bound、termination、close reason 与 action；
- `evidence-view.csv` 按 Evidence × claim 展开 subject、purpose、digest、provenance 与 Criterion assessment；
- `review-view.csv` 按 Packet × Criterion 展开判断、反证、unknown 与正式 Decision；
- `timeline.csv` 按 Trail sequence 给出 transition、对象与理由的可读摘要；
- `meta.csv` 固定 project/objective heads、Trail digest、文件列表与 report schema。

Interaction summary 只保留已解释意图、已确认边界、核实事实、重要质疑/处理结果和 Route-only notes。它不进入
report snapshot，不要求数据库表、索引、签名、恢复协议或逐轮持久化。

Set-like 关系优先展开为多行，不把旧式 compact JSON cell 当正常路径。Renderer 必须使用稳定 UTF-8 CSV、正确
quoting、path traversal 防护与 spreadsheet formula-injection neutralization；测试必须覆盖 `=`、`+`、`-`、`@`
以及前导 whitespace/control character 后出现这些 trigger 的所有文本字段。展示转义不能改变数据库中的原值或
Evidence digest。

Report 在一个 SQLite read transaction 中固定 `project_seq + objective_seq`，把所有表写入 fresh、非业务 identity
的 generation directory；全部文件关闭后，最后用 temp-file + rename 更新 `current.csv`。`current.csv` 只记录
generation relative path、两级 heads 与 report schema。已有 generation 不覆盖、不自动删除；same-head refresh、
schema change 或人工修改恢复都直接生成新 generation。

Renderer 使用普通的安全文件 API，验证 `.mobius/views/` root、拒绝 managed root symlink、编码所有路径组件并
检查最终路径仍在 views 内。它不实现第二套 lock、lease、transaction、scheduler、持久 quarantine 子系统或自动
GC。显式 `report` 若发现 `current.csv` 是 symlink 或错误类型，会在创建新 generation 前用同目录
`.invalid-current-<uuid>` rename-aside 隔离该目录项；rename 不跟随 symlink，也不删除未知内容。这只是无索引、无
恢复协议、无自动清理的一次性数据保全修复。并发 report 允许最后完成者更新 `current.csv`；`mobius report` 读取
current 时比较数据库 heads 与 `meta.csv`。旧 pointer 只有在其 `ReportHeads` 是该 Objective 的真实历史 heads，
且 `meta.csv` 的 Trail digest 精确等于该
heads 对应的真实 Trail prefix digest 时才是 stale。未知或 future heads、meta digest 不匹配及 meta 人工篡改
都归为 incomplete。判定 fresh/stale 前，renderer 会验证八个正文文件均为 UTF-8、CSV 语法有效、表头精确且每行
宽度匹配；无法以当前 snapshot 推断旧 generation 的历史行数或内容。非法 UTF-8 current 归为 invalid，正文或
meta 归为 incomplete；显式 report 会清楚呈现旧状态并生成新的完整 generation。人工修改只影响该份展示。

View 不提供数据库级 crash consistency 或 durability。Crash 最多留下未引用 generation、旧 `current.csv`、
可检测的无效 current 或 interaction temp file；下一次显式 report 从 SQLite 重建 CSV。历史业务查询始终由
Trail 提供；用户可以删除整个 `views/`，之后 CSV 可重建而 interaction summary 可以保持缺失。

自动 view 写入是 SQLite business commit 成功后的派生 presentation effect，不是同一 transaction 的一部分，
也不是 Model transition、Subagent effect 或需要 main Agent 接管的 cleanup：

1. `ActivateObjective` commit 后，presentation adapter 在已有 native session reference 时只用一次目录创建认领并
   初始化尚不存在的精确 run；已存在 run 的 missing current、stale、invalid、incomplete、symlink 或错误类型
   均保持原样，同 request 的幂等 Activate 重放也不能把它们升级为隐式 repair；
2. `ActivateObjective` 或 `ReviseObjective` 接纳后，若调用携带最终交互摘要与 native session reference，且其
   receipt objective head 仍是 live current head，presentation adapter 在 revision-scoped 目录原子写入或覆盖
   `interaction.md`；同 receipt 的幂等重放可以替换摘要，跨 revision 写没有共享目标，旧 revision 重放或延迟写
   不能降级新摘要；失败不回滚 transition，也不进入 Trail/SQLite；
3. 人类显式执行 `mobius report` 时同步生成或刷新，失败必须清楚返回；
4. 只有新提交并产生 `Achieved` 或 `Abandoned` 的 transition 才触发 terminal fanout；幂等 mutation 重放不触发。
   fanout 只对已存在、结构完整、Objective-bound，且旧 `ReportHeads` 与
   Trail prefix digest 都能精确验证的 stale run view 尽力生成 final snapshot；缺失、无效、future、篡改或
   不完整的 view 留给下一次显式 `report` 修复；
5. 其他 transition 不重写完整 CSV；report 通过 heads 判断 stale 并在显式访问时修复；
6. post-commit 自动生成失败不回滚、降级或伪造业务 transition；
7. mutation response 不等待“报表已被人类读取”，也不把 refresh status 注入 Agent Context；只有成功写入
   当前交互摘要时，transport 可以返回其精确 `interaction_path` 供同一 main Agent 交接。

CSV 永远不被 Core、MCP、hook、skill、main Agent 或 subagent 读回作为业务输入，人工编辑也不导入。
`interaction.md` 的唯一 Agent 读取者是设计当前 Stage Route 的 main Agent；它是可删除、非权威、需要复核的背景，
不能成为 transition、Evidence、Decision、proof、completion 或 Map recovery 输入。默认普通 SQL read、其他
mutation response、Subagent envelope 都不返回 view path、文件列表、CSV 内容、refresh task 或生成日志。人类
显式请求 `report` 时才返回有限的 report path、source heads 与 freshness。

## 4. Evidence admission、artifact lifecycle、Packet 与 replay

### 4.1 两种封闭 freeze representation

Core 只实现两条正常路径：

```text
Inline(value)
CoreSnapshot(digest, size)
```

`Inline` 把规范化的完整 observation 直接保存到 Evidence。`CoreSnapshot` 指向 Core-owned content store 中
已经冻结的实际 bytes。实现不提供 external immutable resolver、live locator observation 或第三种扩展分支。

`claims`、`observation` 与 `provenance` 都必须成为 Evidence 的固定值。Locator 只能作为 provenance，不能
单独充当 observation。若候选结果只给出 locator，main Agent 必须先读取并核对实际内容，再选择 Inline 或
CoreSnapshot；否则候选保持模型外状态。

### 4.2 Artifact capture

`capture_artifact` 是 Core delivery service 的通用能力，不认识任何任务、角色或 Packet。它接收 adapter 已
授权读取的实际 bytes，并执行：

Capture 是一次原子提交：main Agent 仅在完整 bytes 能放入当前 MCP 请求与 Context 预算时调用。若完成
Objective 所必需的 blob 超出预算，当前 Objective 必须如实阻塞；不得拆分 blob、绕过 MCP，或以路径引用
代替 bytes。任何 filesystem ingestion 都需要另行设计并审查其读取授权与边界。

1. 在 project-owned `staging/` 中创建临时文件；
2. 写入完整 bytes 并同步文件内容；
3. 计算 digest 与 size；
4. 若目标 digest 已存在，重新核对既有 bytes 与 size；任何 mismatch 都 fail closed；
5. 否则在同一 artifact filesystem 内 atomic rename 到 content-addressed blob；
6. 同步 artifact directory；
7. 返回 typed `CoreSnapshot` reference。

Blob durable 后才允许提交引用它的 Trail event。Capture 本身不是业务事实；进程在 Trail commit 前崩溃最多
留下 orphan blob。

具体同步原语和 SQLite durability 配置必须在支持的平台上形成经过 crash test 的组合。平台无法提供所需
durability 时，CoreSnapshot 路径不可用，不能降级成只记录 live path。

### 4.3 Admission-time validation

`RecordEvidence` 与 `CheckWait` 在持有 project-global writer lock 的数据库 transaction 中：

1. 对 Inline 验证 observation 已完整规范化；
2. 对 CoreSnapshot 重新打开并核对 digest 与 size；
3. 在同一个接纳前态检查 `Fresh` 或 `FreshBatch`；
4. 对批次检查批内 identity 唯一性；
5. 检查 current subject、purpose、Context 与 claims domain；
6. 原子追加 Evidence 与对应 transition fact。

`CheckWait` 的每个 `e ∈ E_b` 都在同一前态 `q` 上满足 `EvidenceAdmission_q(e)`；随后原子记录完整的
`J_b.evidenceSet = W_q(b) ∪ E_b`。任一对象或 artifact 失败时整批回滚。

### 4.4 Packet materialization

`SealAttempt` 只有一条 materialization 路径。调用者提交 current Attempt identity 与 `sealReason`；Application
service 在持有 project-global writer lock 的同一接纳前态中构造唯一的完整 Packet，并形成模型定义的精确
`SealAttempt(P, sealReason)` transition input。Core 随后验证：

- termination 属于模型允许的三类；
- 当前 Attempt 至少贡献一条 Evidence；
- `P.evidenceSet = U_q(P.stage, P.context)`；
- Packet 引用的所有 CoreSnapshot 当前可读取且 integrity matched。

调用者不能提交自选 Packet、选择旧 Trail prefix 或手工省略 Evidence。构造后的 Packet 与 `sealReason` 一起
进入 Trail；Packet 一旦进入 Trail 即不可变，新的 Evidence 只能进入后续 Attempt 与新 Packet。

### 4.5 Reachability、GC 与损坏

`artifact_gc` 与 projection repair 是显式 Core maintenance mutation，同样通过 Core MCP mutation adapter，
并与 admission 共用 project-global lock。

GC 只能删除 Trail 不可达 blob，删除后同步 artifact directory。Trail 可达 blob永久保留。Staging 临时文件、
capture 后未引用 blob 与 crash orphan 可以清理。

若已引用 blob 缺失或 digest/size 不匹配：

- 不从原始 locator、live workspace 或 projection 补写；
- artifact read、Packet materialization、integrity audit 与依赖该材料的新 Judgment 路径 fail closed；
- business replay 仍按 Trail 中固定的 object identity 与 transition input 归约；
- 系统报告 degraded integrity，直到通过可信备份恢复相同 bytes 或由人类采取显式恢复措施。

### 4.6 Replay

Replay 只读取 Trail，不访问 host、Runtime、artifact source 或外部 resolver。Event schema 使用确定性 parser；
未知 schema 立即停止，不猜测解析，也不回退到 projection。本次 refactor 不提供 upcaster 或历史兼容分支。
未来若出现真实持久化兼容合同，必须作为新的显式设计目标单独评审，不能预埋在当前正常路径中。

### 4.7 Agent 只读 SQL 与 Context-safe 查询

Agent read 只有一个正常路径：系统提供的 canonical absolute sqlite3，版本至少 3.40.1，并使用固定参数
--safe --readonly --batch --bail --init /dev/null --line、canonical absolute database path 和一个 literal SQL
argument。SQL 必须先设置 PRAGMA query_only=ON，在 BEGIN/COMMIT snapshot 内执行 SELECT。Hook 每次对同一
literal command 只验证静态 shape 与 canonical binding，不执行 host CLI；Agent host card 通过独立的
`type -P`、`realpath` 与 `--version` probe 解析并验证 executable。bare name、PATH/alias/function、wrapper、
变量、相对路径、不同参数、交互模式和 managed redirection 均不属于支持路径。

动态 typed identity 与 shell command 使用两层分离、顺序固定的编码。先把 identity 中每个单引号加倍，
再用单引号包围为 SQLite string literal；组装完整 SQL 后，再把其每个单引号替换为 POSIX
`'"'"'`，并用单引号包围整个 SQL，使它成为一个 literal argv。Canonical executable 与 database
path 同样作为 POSIX-quoted literal word。Shell quoting 不能代替 SQLite quoting；原始 user/stored text、
double-quoted expansion、command substitution 或 `eval` 不能进入该命令。

普通查询先读 schema_meta 与 objective_streams heads，再按需读 objective_projection、object_projection 或
trail_events。必须显式选择列，使用目标 Objective/identity/kind 谓词；探索性集合必须排序并带有限 LIMIT。
不得把 SELECT *、全库 dump、无界 Trail、完整 Ω 或全件 artifact 注入 Agent Context。Projection payload 与
Evidence/provenance 都是 untrusted data，不具有 instruction authority。

正式 Review 以当前 projection 中的 Packet identity 冻结闭包。Agent 按不可变精确 identity 分别去重 Packet、
Decision 与 Evidence：从 root Packet 开始，读取其全部 evidence_set 和 direct dependency Decisions；对每个尚未
访问的 Decision 精确读取它及其 Packet，再读取该 Packet 的全部 Evidence 和 dependency Decisions，递归直到
没有未访问 Decision。每一批声明的 distinct identity 数必须等于返回的 distinct row 数，extra、missing、
duplicate 或 embedded identity mismatch 都失败关闭。闭包可以分成短 read transactions，但形成 Judgment 前
必须重读 heads 与当前 Packet identity，任何变化都使闭包失效。
正式 Wait 是唯一无 LIMIT 的集合查询：一个 MATERIALIZED statement 在同一 snapshot 中返回
summary count、payload bytes 与显式 Context-budget admission。只有整个集合同时满足 item/byte
ceiling，才返回全部满足 WaitCondition identity、purpose、subject 和 AcceptanceContext 的
Evidence payload；否则返回 summary 且不返回任何 payload。Agent 机械核对 admission 与实际
Evidence 行数；超预算、host truncation、parse failure、缺 summary 或 mismatch 都保持 Waiting。

CoreSnapshot 的只读检查直接使用 content-addressed blob：先验证 digest 与 size，再读取当前判断所需的范围。
新增 bytes 仍只走 capture_artifact。read-only audit 走 mobius audit CLI；MCP audit 只接受显式 projection
rebuild 或 artifact GC maintenance。

该设计不增加 view/table、SQL parser、query builder、ORM、cursor、chunk protocol、第二 read service 或
兼容 shim。直接 SQL 没有服务端 row/byte cap，这是公开 residual risk；通过最小 SELECT 与普通 LIMIT 控制，
而不是伪装成机械安全保证。

## 5. Core API、host admission 与 mutation adapter

### 5.1 Application service

所有 Core adapter 汇聚到同一个 service：

```text
project_init
capture_artifact
apply_transition
audit
rebuild_projection
artifact_gc
report_snapshot
```

`report_snapshot` 是 presentation adapter 专用的只读 query：它在同一 SQLite read transaction 中固定两级 heads
并返回 format-neutral、无 session/path/CSV 字段的 typed rows。它不作为 Core MCP method 暴露，也不向 main
Agent 返回 CSV 概念；只有 presentation adapter 把这些 rows 编码为人类视图。

Application service 不提供 Agent read facade。内部可以有只服务 Stop hook 或 presentation 的窄只读函数，
但不得重新暴露通用 query enum、SQL proxy、对象分页、artifact range 或 next-action recommendation。

`apply_transition` 只接受一个具体 typed mutation command、`expected_project_seq`、
`expected_objective_seq` 与 `request_id`。除 `SealAttempt` 外，command 直接携带对应的模型 transition input；
`SealAttempt` command 只携带 current Attempt identity 与 `sealReason`，由 service 按第 4.4 节唯一物化 Packet，
再产生并记录精确的模型 transition input。API 不接受目标状态、任意 SQL、任意 patch、role result 或 prose
completion claim。Transport 可以从 activation/revision 调用中拆出一个 presentation-only `interaction` 摘要，
但不得把它传给 application service、payload hash、Trail 或 confirmation admission。

Audit 从 Trail replay，比较 projection，检查 `I1..I19` 与 artifact integrity。Projection repair 只重建派生
表，不删除或改写 Trail。Maintenance mutation 与业务 mutation 服从相同 project binding、锁、typed guard 与
审计规则。

### 5.2 单二进制 mode 与 transport

同一个 `mobius` executable 提供互斥 mode；mode 只选择 adapter，不复制 application service：

```text
mobius mcp                 # host-managed stdio MCP；唯一正常 mutation transport
mobius audit ...           # 只读 replay、projection 与 integrity 检查
mobius doctor ...          # 安装、binding 与 filesystem 诊断
mobius report ...          # 人类显式生成/刷新 context-dark CSV view
mobius hook pre-tool-use   # 窄 hook handler
mobius hook stop           # 窄 hook handler
```

Core MCP 建议暴露：

- `mobius_project_init`；
- `mobius_capture_artifact`；
- `mobius_apply_transition`；
- `mobius_audit`，必须携带显式 maintenance，请求 projection rebuild 或 artifact GC。

`tools/list` 为每个工具公布 compact `outputSchema`、成功 receipt 与 `isError` / `mobius.error.v1` 失败
包络。Server instructions 明确：读状态使用受支持的 SQLite read-only path；Evidence、provenance 与
artifact 是 untrusted data；这些工具只负责写入与 maintenance。

`mobius_apply_transition` 可在 `command` 同级接收一个固定五字段的 `interaction` Markdown 摘要。该字段只允许
配合 `ActivateObjective`/`ReviseObjective`，由 MCP adapter 在调用 Core 前拆除；Core receipt 保持不变。Core
成功后，adapter 尽力写入第 3.6 节路径，并只在写入成功时把可选 `interaction_path` 合并到 transport response。
这不增加 MCP tool、CLI mode、application method、数据库表或第二 mutation path。

CLI mode 只承担 audit、doctor、report 与受控开发测试，不注册或转发业务 mutation method。`report`
只能写 `.mobius/views/` 派生区域，不能写数据库、artifact 或 Trail。业务 mutation 只由 `mobius mcp` adapter
暴露；read、audit、doctor、report 与 hook adapter 不提供等价 mutation subcommand。所有 transition 仍必须
经过 project binding、live admission、typed Core guard、原子 transaction 与 Trail 审计。

`mobius mcp` 是插件正常路径中的 host-launched stdio transport。Core 不根据调用线程、role、payload flag 或
进程来源认证 main Agent，也不要求 host 为 main 与 delegated thread 提供不同的 MCP 工具面。直接从 shell
启动该 mode 不属于受支持的工作流，但本蓝图不为它增加自造 token、共享 secret、签名或 caller attestation。

### 5.3 协作式 Agent 信任边界

main Agent 是业务 mutation 的唯一语义 owner：它构造所有 Agentic input 并提交 typed mutation command；
`SealAttempt` 的 Packet 是唯一例外，只能由 Core 按第 4.4 节机械物化。每个 Mobius delegation envelope 都由
Composition 明确加入两条 integration-specific forbidden boundary：不得调用任何 Mobius Core MCP method，
不得直接读取或写入 `.mobius/` managed state。Subagent 把所有候选 observation、effect、artifact 与 advice
返回 main Agent。该边界依赖 Skill、任务 envelope 与模型遵循，不是针对恶意 caller 的安全边界；Runtime
无需机械隐藏这些工具。

Main Agent 只有在为 current `SeekingRoute` Stage 设计 Route 时，可以读取一个按第 9.1 节解析出的
`interaction.md`；它仍不得写 `.mobius/`、读取其他 view 作为业务输入，或把 interaction 内容传给 subagent。

Runtime 可以向 main Agent 与 delegated thread 暴露相同工具，也可以让它们继承相同 sandbox 与 permission。
实现不建立 per-thread capability、caller identity、service handle 隔离或角色专用 sandbox。Core 对每个请求
一视同仁地执行 typed guards；这些 guard 能阻止非法状态转移，但不证明调用者一定是 main Agent。

本信任模型明确接受一项残余风险：若 subagent 违反 Skill 或被攻陷，它可能尝试直接调用 mutation transport。
这类对抗性行为不在 Mobius 的威胁模型内。Trail、audit 与窄 hooks 用于保持状态可检查并减少意外旁路，不承担
main/subagent 身份认证。

### 5.4 Human confirmation admission

`ActivateObjective`、`ReviseObjective` 与 `Abandon` 等模型要求人类确认的转移使用一条协作式、typed
confirmation 路径。用户明确确认完整 action 与 payload 后，main Agent 构造 `H^{confirm}` 或
`H^{abandon}`，并连同 mutation command 提交。Core 在 live admission 中验证 confirmation 精确绑定：

- 当前 project 与 Objective；
- 当前 project 与 Objective 两级 head；
- action；
- 向用户展示并获确认的完整 typed payload。

confirmation 是 main Agent 在明确人类确认后构造的语义 transition input，不是 Runtime principal attestation、
签名或 host-issued capability。Core 检查 typed binding、两级 heads 与当前状态，不认证调用线程；可附
opaque、reducer-inert host/UI audit reference，但该 reference 不是接纳前提。Reducer 与 replay 不重新访问
host。缺少明确用户确认、payload/head 绑定不完整或 stale 时 fail closed；未经确认的 main Agent 自述、CLI
flag 或 Subagent 输出不能代替人类确认。

Activation/revision 前，Copilot 先以专家视角调查可发现事实、质疑模糊或冲突表述，并每轮只解决一个会改变
ObjectiveSpec/Map 的关键问题；清晰后先展示解释摘要，再执行上述 exact typed confirmation。人类确认
ObjectiveSpec，main Agent 独立设计 Map 和 Route。`interaction` 是已展示理解的派生摘要，不扩大或替代
confirmation authority。

Subagent Runtime success、Judge result、模型数量与 role 字段都不是 Core authority，也不进入 live admission。

## 6. Subagent Skill：独立角色、任务、结果与 effect

### 6.1 Packaging

Subagent 模块保持 instruction-first：

```text
plugins/mobius/
  skills/
    mobius-subagent/
      SKILL.md
      agents/
        openai.yaml
      references/
        role-profiles.md
```

`SKILL.md` 只保留设计不变量、角色选择、Driver 原生调用、basic envelope、公共 result、消费检查与生命周期。
五种完整角色模板按需从一层 reference 读取。只有确定性校验确有价值时才增加 script。

该目录不得依赖 Rust Core crate 或 module，不得复制 Objective、Trail、Core path 或 API schema，也不得建立
ledger、registry、queue、heartbeat、memory 或 Runtime mirror。Subagent Skill 只表达通用的 downstream
ownership 边界；“不得调用任何 Mobius Core MCP、不得访问 `.mobius/`”由 Composition 按第 5.3 节加入每个
Mobius delegation envelope。该协作式边界不要求 Runtime 从 subagent 工具面机械移除相应能力。

### 6.2 Runtime ownership

Agent/thread、turn、item、工具、权限、模型与 usage 以 Codex Runtime 官方对象为唯一来源。Subagent Skill 不
重新序列化或持久化这些事实。main Agent 在当前任务内保留 Runtime identity 与 delegation baseline 的临时
关联；任务结束后不把它升级为业务 identity。

Driver 只使用当前 host 支持的原生 Subagent workflow，并继承 host 实际 sandbox 与 permission。Driver 是
委托语义角色，不要求证明某个内建 agent identity；Runtime 负责选择和执行实际 profile。Subagent Skill
不能扩大权限，Composition 也不主动向任何角色传递 Core handle 或 mutation instructions。即使 Runtime 的
继承工具面包含 Core MCP，Driver 仍遵守 envelope 中的 forbidden boundary；实现不为此增加线程级 capability 层。

### 6.3 Task 与 result

每个任务都自包含 `background`、非空 `objectives`、forbidden-first boundaries、所选 role input、完整
output format 与非空 DONE conditions。Envelope 是委托语义，不是 Runtime transport protocol。

每个 task 还要求一个有限 `result_budget.max_public_result_bytes`，约束完整序列化公共结果。每个 result
只返回一份紧凑 synthesis、公共执行闭合字段、唯一 `role_output`、effect inventory、artifact inventory、
不确定项与 blocker。原始日志、大型内容和超额详情留在 Runtime
thread 或可核查 artifact，通过 locator 按需获取；不在 summary、objective result 与 role output
重复同一内容。`status=completed` 只表示本次执行正常返回，不表示任何下游 Objective、
Criterion 或 Stage 完成。

Artifact locator 只说明 main Agent 去哪里核查；它不证明内容冻结。Effect 声明只报告真实世界中已经发生或
尝试的副作用；它不自动成为 Evidence。

### 6.4 Effect contract

允许副作用的任务必须逐项声明 target、operation、authorization、status、before/after、provenance、
verification、unexpected impact、residual risk 与 cleanup responsibility。

未授权、授权不明确、失败、partial、rolled back 与 cleanup pending 都必须如实返回。One-shot subagent 不能
成为后续责任人；main Agent 必须接管仍待处理的 cleanup。

## 7. Judge freeze、Runtime 生命周期与委托并发

### 7.1 Judge material freeze

Judge 的 `materials[].freeze` 只属于一次委托。Composition 可以把 CoreSnapshot 的相同 bytes 作为材料，
但必须创建 Subagent-local material id、freeze declaration、questions、criteria 与 required coverage。

Judge 必须逐项返回 freeze check 与实际 coverage。只有 freeze matched 且 required coverage complete 的材料
可以支撑确定性 assessment；partial、stale、unverifiable 或 inaccessible 必须使相应结论和整体 disposition
变成 `inconclusive`。

Judge freeze 与 Model `FrozenEvidence`：

- identity 不同；
- schema 不同；
- admission 时点不同；
- owner 不同；
- 不互相证明。

Judge 的 matched/coverage 只产生 advice。它不能建立 Evidence、ReviewDecision、proof 或 transition；一个
Evidence digest 也不能自动证明 Judge 已完整审查材料。

### 7.2 Lifecycle

main Agent 构造完整任务，通过原生 Runtime spawn，直接消费最终 output、items、status 与 usage，并关闭已
完成、失败或不再需要的 thread。Follow-up 只用于同一 envelope、baseline 与授权边界内的澄清；目标、角色、
授权、冻结材料或 baseline 改变时创建新任务。

### 7.3 Concurrency

- 独立只读调查可以并发；
- 多个 Judge 可以独立审查同一组冻结材料；每个新 Judge 必须对应尚未覆盖的问题或
  failure model，main Agent 在有限 delegation/result budget 内选择最少的有用 fan-out 并只返回一份自己的综合；
- 修改范围重叠或竞争同一外部对象的 Driver 必须串行；
- Verifier 在待验证 effect 已发生并稳定后启动；
- subagent 工作可以并发，main Agent 对 Core 的提交严格串行；
- baseline 过期的结果不能按旧前提接纳，但已经发生的 effect 仍须核查、清理或回滚。

## 8. Main Agent Composition：baseline、消费、转义与 Judgment

### 8.1 Composition 不是共享 adapter

Composition 是 main Agent 的编排责任，不是一套由 Core 与 Subagent 共同 import 的 schema。它可以在 skill
instruction 中描述检查顺序，但不能定义 `CandidateInput` registry、跨模块 identity、共享 task hash 或自动
field mapping。

Delegation baseline 是 main Agent 从当前只读 SQL observation 与开放世界事实转义出的普通冻结摘要。它只固定会改变
委托结果适用性的事实或材料版本，不复用 Core object schema 作为 Subagent 契约。

### 8.2 唯一消费路径

```text
read-only SQLite + current project/objective heads
→ main Agent 构造普通 delegation baseline
→ 可选：使用 Subagent Skill 调查、执行、验证或审查
→ main Agent 消费 Runtime result
→ 核查 baseline、freeze、effects、provenance、coverage、unknowns 与 cleanup
→ main Agent 构造完整 Model object、Judgment 或 `SealAttempt` command
→ `SealAttempt` 时 Core 唯一物化 Packet；其他 command 直接提供 Model input
→ Core 在 latest head 上重新运行 guard
→ accept 或 fail closed
```

Route 设计有一条更窄的可选背景路径：main Agent 先固定 current ObjectiveSpec、Map、Stage 与 StructuralContext，
再按第 9.1 节读取至多一份匹配的 `interaction.md`，复核其中会影响 Route 的事实，最后独立构造 `Route`。
该文件不进入上述 Evidence/Judgment 消费路径。

任何 `status`、`objective_results`、`role_output`、`recommended_disposition`、effect 或 artifact locator 都不能
自动映射为 Evidence、Decision、proof、completion 或 transition。

### 8.3 Candidate observation 到 Evidence

main Agent 必须：

1. 核查实际 observation，而不是只转录 subagent conclusion；
2. 确定 current Attempt 或 WaitCondition subject；
3. 选择正确 purpose 与当前 Context；
4. 明确 claims domain；
5. 把 observation 与 provenance 固定为 Inline 或 CoreSnapshot；
6. 提交完整 Evidence；
7. 接受 Core 对 `EvidenceAdmission_q` 的最终机械判定。

Effect envelope 本身不是 observation。若 Driver 修改了世界，main Agent 要检查实际 diff、命令结果或外部对象，
再把观察到的后态转义为 Evidence。

### 8.4 Advice 到正式 Judgment

main Agent 可以不调用 Judge，也可以消费零到多个彼此独立的 Judge advice。无论使用多少 advice：

- main Agent 自行检查 Packet、完整 Evidence、反证与 unknown；final-integration 还要检查完整
  `DependencyView`；
- main Agent 独立形成 `ReviewDecision`；
- main Agent 独立形成 CheckWait 的 `J_b`；
- 票数、模型数量、Runtime success 与 Judge disposition 都不能替代正式判断。

正式 Judgment 进入 Core 前不保留模型外 role、thread 或 task lifecycle。Core 只验证其 typed completeness、
适用性与状态不变量。

Main Agent 必须完成第 4.7 节的 exact Review identity closure；Wait 必须由同一 snapshot 的 MATERIALIZED
查询同时返回 count、payload bytes 和 admission；只在 admission 成功时返回全部匹配 Evidence，
且实际行数精确等于 count。超预算、host-truncated output、缺失 summary、旧 snapshot、identity 漂移
或 count mismatch 都不能支撑正式 Judgment。

### 8.5 Stale 与 failure

若返回时 current subject、Acceptance Context、head 或冻结材料版本已变化，结果只能保留为模型外线索，或基于
新 baseline 创建新任务。不得修改旧 result 使其看似适用于新 Context。

Subagent timeout、failure、boundary violation、inconclusive Judge 或 unavailable material 不推动 Model 状态。
已发生 effect 不会因结果被拒绝而消失；main Agent 仍负责检查与 cleanup。

## 9. 总装主循环、恢复与 transport 接线

### 9.1 Model skills

`mobius-copilot` 与 `mobius-loop` 属于 Composition shell。前者独占人类授权的 Objective contract action
（activate、revise、abandon），并为 `MappingReason::Initial` 与 `MappingReason::SpecRevised` 安装 Map；后者
只从已经 active 的 Core 状态继续执行，并为 `MappingReason::Remap` 与
`MappingReason::WaitRevealedDrift` 安装 Map：

- 只在用户明确指定 Mobius Objective 时触发；
- `mobius-copilot` 与 `mobius-loop` 在 host metadata 中禁用 implicit invocation；已经显式进入
  `mobius-loop` 后，main Agent 可以按工作需要选择可发现的 `mobius-subagent`，无需用户再次调用 Skill；
- 通过第 4.7 节的只读 SQL 获取最小 state-relevant facts，不要求 Core 推荐 next action；
- 相同 heads 的新 observation 替代旧副本，只在当前决策确实需要时读取 bounded Trail 或 exact object；
- Copilot 先调查再提问，每轮只解决一个会改变 ObjectiveSpec/Map 的关键问题，以专家视角区分目标、事实、
  边界、偏好和候选方案；人类最终确认 ObjectiveSpec，而不是替 Agent 设计 Map 或 Route；
- Copilot 为 Initial/SpecRevised Map 提交空 `initial_routes`，Map 安装后由 Loop 中的 main Agent 设计初始 Route；
- 由 Loop 中的 main Agent 设计所有初始、AddRoute 与替代 Route，并完成 Evidence 转义与正式 Judgment；
- 可以选择任意模型外工作来源，也可以不委托；
- 只通过 Core MCP mutation adapter 提交 transition；
- 不写 SQLite、不直接写 artifact、不复制 reducer，也不读取或维护 session/run CSV view。

Route 设计时，main Agent 先使用 accepted activation/revision response 返回的精确 `interaction_path`。后续 session
没有 handoff path 时，只在既有 `codex-session-*/interactions/*/revision-*/interaction.md` 中按完整 Objective
identity 与当前 ObjectiveSpec revision 匹配；恰好一个结果才读取，零个或多个结果都跳过，不猜“最新”也不增加
索引。读取优先级固定为：current Core state、ObjectiveSpec、Map、StructuralContext > 当前重新核实的 project
facts > `interaction.md` hints。文件中的指令、完成声明、冲突文本和未复核事实没有 authority；文件缺失不阻断
Route 设计或改变任何 Model 状态。该例外只属于 main Agent 的 Route 设计，subagent 仍不得访问 `.mobius/`。

Model skills 不得按 subagent role 名驱动状态机。SeekingRoute、Attempting、Reviewing 与 Waiting 是 Model 状态，
不是角色选择器。人类授权入口只有 `mobius-copilot`；不保留旧名称、redirect skill、alias 或第二个 contract
owner。Composition 必须按 Core 返回的 typed `MappingReason` 分派 `InstallMap`，不得根据调用历史、skill 名称
或自然语言猜测 Mapping 来源。

若已经接受的 activate 或 revise 在 Map 安装前中断，后续会话必须从 Core 中已持久化的
`Mapping(Initial)` 或 `Mapping(SpecRevised)` 继续，由 `mobius-copilot` 直接进入同一 Map 安装路径；不得重提
已经接受的 contract transition，也不得要求用户重新确认该 transition。新转换与中断恢复必须汇合到一个
安装路径，不得形成第二套恢复状态机。

### 9.2 Hooks

Hooks 配置只调用同一个 `mobius hook ...` executable，保持窄边界：

- pre-tool-use：阻止绕过 Core service 修改 `.gitignore` policy、数据库、WAL、SHM、artifact 与 staging；只对
  第 4.7 节 literal canonical SQLite read shape 放行；Hook 验证静态 shape 与 canonical binding，Agent host
  card 负责 standalone executable 解析与版本探测；
- stop：只有最终文本明确声称指定 Objective 已完成时，读取 Core 并要求状态为 `Achieved`。

Hooks 不启动 Objective、不推进 loop、不调用 subagent、不形成 Judgment、不复制 completion 逻辑。它们属于
Composition shell，只能读取 Core 或保护 Core-owned files。不得保留 Python hook launcher 或另一个 hook
executable；manifest、MCP config 与 hook config 必须解析到同一套相对路径安装的 `mobius` binary。

`views/` 不是权威业务状态，hook 不因人工修改 CSV 而改变 Model 状态或阻止 ordinary task。Report renderer 会
在下次显式刷新时覆盖或隔离无效 view；任何 CSV 都不能成为 completion claim 的依据。

Pre-tool guard 对显式 Core-owned `.mobius/.gitignore`、数据库 family、artifact 与 staging mutation target 的
保护不依赖 project binding；`.gitignore` 的精确 `*` policy 使自身与其余 private cache 在 repeated default
`git clean` 下保持 ignored；`views/` 仍是上述
例外。项目 root/ancestor 级 destructive scope 按每个精确 effective cwd、tool `workdir`/`cwd`、静态 wrapper cwd
override、Git `-C`/`--work-tree` scope、顺序 `cd` 候选和绝对 mutation target 分别发现 regular、非 symlink 的
`.mobius/mobius.sqlite3` binding；同时出现且不同的 `workdir` 与 `cwd` 必须作为两个候选检查，hook cwd 属于项目 A
时不能遮蔽 tool cwd 或目标所属的项目 B。静态 destructive ancestor scope 还必须以 no-follow traversal 检查其
descendants；发现 binding 或因 I/O 无法判定时阻止，非目录、不存在、marker wrong-kind 与不跟随的 symlink subtree
不形成 binding。显式要求跟随 symlink 的 destructive mode 遇到相关 symlink 时 fail closed。未绑定目录中且不进入
任何绑定项目的 broad filesystem operation 属于 ordinary task，必须允许，不能仅因 cwd 看似 project root 而
推断 Mobius ownership。结构化 mutation 的 `path`、`file_path`、`target`、`target_path` 与相应 destination 字段
适用同一显式 Core-owned target 保护。

Bound-root `git clean` 的 `-x`/`-X` 与任何以 `!` 开头的有效 `-e`/`--exclude` pattern 都可能把 private cache
重新纳入删除范围，必须 fail closed；正向 exclude 与 default clean 保持 ordinary，`-n`/`--dry-run` 不形成 mutation。
上述 ignored-state mode 带静态 pathspec 时，ordinary 与 `..` scope 按 effective cwd 解析，`:(top)`/`:/` scope
按实际 Git worktree root 解析；命中 Core-owned target 时阻止，未命中的静态 subdirectory/file pathspec 保持
ordinary，包括 bound root 上只选择普通文件的 clean。静态全局 `--icase-pathspecs`、
`GIT_ICASE_PATHSPECS=...` assignment 与未建模的 inline pathspec magic
只在其 resolved containing scope 内保守处理：先应用 pathspec body 开头的 root/parent traversal，再从所得
scope 保护全部后代，不信任后续大小写敏感的名称片段，因此嵌套 Core-owned target 也不能绕过。已识别的 dry-run
返回明确的 non-mutation，不得再落入通用 mutation fallback。这里不扩展为通用 Git pathspec engine。

Shell-shaped input 只对静态 builtin `cd` 传播 cwd：它可以直接出现，或只经过一个或多个 literal POSIX
`command [-p] [--]` 前缀，并可带 POSIX assignment、重复或组合的 `-L`/`-P` 和 `--`。最后一个 mode option
生效，`-P` 只在现存目录可解析时使用 physical canonical path，`-L` 保持词法路径。一个共享的有界执行流按
pipeline unit 与成功/失败分支分析 mutation 和 destructive scope：`&&` 只推进成功分支，`||` 保留短路成功分支
并保守检查原 cwd 与候选新 cwd，pipeline member 使用 pipeline entry cwd，background `&` 后恢复整个 AND-OR
list 的 entry cwd，`;`/newline 开始新的前台 list。连接符后的 newline/CRLF 是 continuation；合法尾随 `;`、
newline 或 `&` 不丢弃其前面的命令分析。静态 redirection operator 与 operand 从 argv 中独立消费，destination
仍作为 mutation target 检查，因此 leading 或 interspersed redirection 不能隐藏后续 operation/operand。
path-qualified `cd`、`env`/`sudo`/`nohup` 包装的 `cd`、path-qualified
`command`、alias、function、`eval`、grouping 与动态 target 不被解释为 builtin cwd transition。已知确定性
read-only command（包括 `file`）不产生 mutation target。该 lexical guard 是协作式安全网，不是 shell
interpreter 或 hostile-process sandbox。

实际交给文件系统的 tool/wrapper cwd、Git scope 与静态 destructive target 使用同一个窄路径规则：现存路径可解析时
采用 physical canonical path，无法解析时回退 lexical path；最终项是 no-follow symlink 时保留 lexical path，由命令的
symlink mode 决定是否 fail closed。该规则不推演动态路径，也不建立第二套 cwd provenance 状态机。

### 9.3 Crash 与恢复

- SQLite commit 前崩溃：业务 transaction 回滚；durable orphan blob 可由 GC 清理；
- SQLite commit 后响应丢失：相同 request id 与 payload 返回已提交结果；
- stale head：返回 conflict，main Agent 重读并重新判断；
- projection mismatch：停止 mutation，audit 后只重建 projection；
- referenced artifact 缺失：报告 degraded integrity，阻断依赖材料的读取与 Judgment；
- database corruption：显式失败，不从 projection 或其他文件返回成功形结果；
- subagent 不可用：Model 状态保持不变，main Agent 可以直接工作、等待或报告真实 blocker；
- report 中途崩溃：业务状态不受影响；旧 current、未引用 generation 或无效 current 由下一次显式 report 检测并重建。

### 9.4 正常路径独立性

必须分别验证两条 E2E：

```text
Human → Main Agent → Model Core
```

以及：

```text
Human → Main Agent → optional Subagent
                   → Main Agent translation → Model Core
```

第一条路径证明 Subagent 不是 Model 的必要依赖；第二条路径证明委托结果必须返回 main Agent，不能直接进入
Core。

## 10. 实施阶段、验证门禁与非目标

### 10.1 Phase 1：Rust binary skeleton 与领域内核

- 建立一个 Cargo package、一个 `mobius` binary target、内部 module 边界与锁定依赖；
- 建立 `mcp/audit/doctor/report/hook` mode dispatch，尚未实现的 mutation 必须显式 fail closed；
- 完成十一类对象、状态、transition input 与 identity mapping；
- 实现纯 reducer、派生查询与全部 guards；
- 用负向 table tests 覆盖模型第 3.4、3.5 节的全部 Map 结构约束；
- 用 table-driven tests 覆盖模型第 10 节；
- 用生成式状态机测试覆盖 `I1..I19`；
- 完成纯内存 Trail replay 与 Manifest 等价测试。

### 10.2 Phase 2：Store、artifact 与 Core API

- 建立 SQLite schema、project binding、project-global single-active constraint；
- 实现 append transaction、idempotency、projection 与 rebuild；
- 实现 capture、durability、admission-time validation、integrity 与 GC；
- 实现固定 heads 的 report snapshot、unique generation、current pointer、精确历史 heads + Trail prefix
  digest stale detection 与 post-commit presentation effect；
- 实现 Core service、四工具 stdio MCP 与 audit/doctor/report CLI；
- 实现第 3.3、4.7 节的直接 SQLite read contract、逐次 Hook admission、exact Review closure 与
  count/byte-admitted Wait query；删除 read facade、cursor/chunk 与 artifact range transport；
- 用 service 与 MCP protocol 负向测试证明 `SealAttempt` command 只接受 current Attempt identity 与
  `sealReason`，拒绝 caller-supplied Packet、Trail prefix 或 Evidence selection，并在同一 locked admission
  prestate 唯一物化完整 Packet；
- 覆盖 SQLite 与 artifact 每个关键 crash point。

### 10.3 Phase 3：独立 Subagent Skill

- 实现薄 `SKILL.md` 与按需 role profiles；
- 验证 basic/result envelope、effect inventory 与 Judge coverage gates；
- 验证各受支持 host 的原生 Subagent 生命周期、Driver 语义与权限继承，并用负向 lifecycle tests 证明
  spawn、配置、Runtime 与权限错误均如实失败；
- 完成 `Mobius-subagent.md` 的十四项验收，包括有限 result budget、overflow 与去重综合；
- 静态证明 Subagent package 不依赖 Core，也不包含任何 downstream-specific API、path 或 schema knowledge。

### 10.4 Phase 4：Composition 与总装

- 实现 `mobius-copilot` 与 `mobius-loop` 使用新 Core API；
- 实现 baseline、结果消费、Evidence 转义与正式 Judgment 检查清单；
- 实现 Copilot 专家式多轮意图澄清、空 initial Routes 的 Map 交接，以及 Loop 对所有 Route 的设计责任；
- 由现有 presentation adapter 在 accepted activation/revision 后写入简单 `interaction.md`，并实现 Route 设计时
  的窄只读规则；
- 更新窄 hooks；
- 完成有/无 Subagent 的两条 E2E；
- 完成 forbidden-import、Subagent-to-main 正常返回路径与 human confirmation binding tests；
- 用 envelope negative tests 拒绝缺少“任何 Mobius Core MCP 均禁止”或“`.mobius/` 读写均禁止”任一边界的
  Mobius delegation；
- 静态与 E2E 证明 model skills、默认 MCP response 和 Subagent envelope 不暴露 CSV view；只有携带交互摘要且
  写入成功的 activation/revision response 可以返回 `interaction_path`；
- 删除 Python runtime、launcher 与 dependency path；
- 更新 references、manifest、MCP/hook 相对启动路径、release docs 与发布门禁。

每个 Phase 只有一个正常路径。尚未完成的后续 Phase 不通过 fallback、shim、alias 或双引擎伪装完成。

### 10.5 分层验证矩阵

| Layer | 必须证明的性质 |
|---|---|
| Rust artifact | 一个 Cargo package 只产出一个 runtime executable；Core mode 无 Python 依赖；Agent read host 有 SQLite 3.40.1+ prerequisite |
| Module boundary | domain 无 transport/infrastructure/Runtime/Subagent 依赖；Subagent resources 无 Core schema/import |
| Core types | 十一类对象、identity、结构相等与所有 transition input 完整映射 |
| Reducer | deterministic、全部 transition、`I1..I19`、terminal rejection、Route rejection 唯一事实来源 |
| Trail/SQLite | replay、projection rebuild、project order、幂等、stale conflict、原子 rollback |
| Project | `|MobiusSQLite(project)| = 1`、`.mobius/` 是唯一私有状态边界、无拆库或 home/XDG/global fallback、跨项目隔离 |
| Bootstrap | 并发 init 单一 database/binding、symlink 拒绝、response-loss retry、before/after commit crash、partial layout 检测 |
| Evidence | Inline/CoreSnapshot freeze、接纳前态、批内唯一、Context、claims domain |
| Artifact | durable-before-Trail、existing digest mismatch、orphan GC、reachable retention、missing fail closed |
| Packet/Review | Core-only Packet materialization、精确 `U_q/W_q`、完整 Criterion domain、矛盾 accept 拒绝、material integrity |
| Agent SQL read | 五表 schema 可观察；literal safe/readonly/query_only command；普通查询有限；Review exact closure；Wait 同 snapshot count/bytes admission 后全量或零 payload |
| Subagent | 十四项独立验收、有限 result budget/overflow/去重综合、无 downstream-specific API/path/schema knowledge、Judge advisory-only、effect 完整声明 |
| Driver | 只走当前 host 的原生 Subagent workflow；不要求 agent identity attestation；spawn、配置、Runtime 或权限错误如实失败 |
| Composition | stale baseline 拒绝、实际 effect 核查、双 freeze 隔离、正式 Judgment 归 main Agent |
| Transport | MCP/CLI/hook 共用一个 service 与 binary；CLI 无业务状态 mutation surface；presentation adapter 是唯一派生 view writer |
| 协作式 Agent 信任边界 | 每个 Mobius delegation envelope 都禁止任何 Core MCP 与 `.mobius/` 直接访问；Core guard 不依赖 caller identity；对抗性 subagent 明确超出威胁模型 |
| Human confirmation | Copilot 先调查、逐个澄清关键分歧；main Agent 在明确用户确认后提交的 typed confirmation 精确绑定 action、payload 与两级 heads；缺失、不完整或 stale 时 fail closed |
| Hooks | 显式 Core-owned target 始终受保护；bound-project root/ancestor guard 与 ordinary unbound task 边界准确 |
| Human view | CSV path containment、pinned heads、unique generation、正文 UTF-8/CSV 结构校验、精确历史 heads/digest stale detection、formula neutralization；interaction path containment、revision 隔离、固定 Markdown sections 与原子覆盖 |
| Agent context | 默认 SQL/mutation/skill/subagent 输出只含当前动作所需的最小 material；无全量 Trail/大对象/整件 artifact/重复 fanout、CSV、refresh task 或 report log；仅 Route 设计读取至多一份 interaction；正式 Wait 超预算只返回 summary 并保持 Waiting |

关键 E2E 至少覆盖：

- `Activate → Map → Route → Attempt → Evidence → Review → Achieved`；
- `retry / replace / wait / remap / revise / abandon`；
- `Decision(replace)` 与 `CheckWait(new_route)` 都拒绝当前 Route，其他 Review/CheckWait direction 都保持 Route status；
- 不使用 Subagent 的完整 loop；
- 使用任意模型外候选结果并经 main Agent 转义的 loop；
- stale result、partial effect、unauthorized effect 与 cleanup pending；
- referenced artifact missing、crash orphan 与 projection rebuild；
- 跨 Objective 改变 project head 后，旧 `expected_project_seq` 被拒绝；
- 使用 Subagent 的 E2E 中，候选结果返回 main Agent，并仅由 main Agent 构造和提交 typed mutation command；
  `SealAttempt` Packet 仍仅由 Core 物化；
- 在不安装 Python、virtualenv 或 pip、但提供受支持 SQLite CLI 的 clean environment 中完成两条 E2E；
- 对每个受支持 target 构建 release artifact，并证明其只有一个 Mobius executable entrypoint；
- 同一 heads 生成等价 CSV rows；current/meta 只把精确历史 heads + Trail prefix digest 识别为 stale，并把
  future、篡改、缺失与不完整状态留给显式 repair；人工修改不影响业务状态；
- report crash 或并发 last-writer 不影响 SQLite/Trail，显式 report 可以重建有效 generation；
- 显式 `mobius report` 可生成完整人类视图，而完整 Mobius loop 的 Agent Context 不包含 view 内容或维护任务。
- accepted activation/revision 写入当前 session/slug/revision 的 `interaction.md`；同 receipt 幂等重放可替换
  摘要，不同 revision 没有共享写目标；旧 revision 重放或延迟写、rejected transition、缺失 session 或
  presentation 写入失败均不覆盖新 revision 摘要；删除或修改该文件不改变 Trail、replay 或 completion；初始
  Route、AddRoute 与替代 Route 都由 Loop main Agent 设计，且 typed state 与重新核实事实优先于 interaction
  hints。

Rust 层的最小门禁是：锁定依赖构建、format、lint、unit/integration/property tests、MCP protocol tests、SQLite
crash tests、artifact durability tests、CSV safety 与 context-surface tests。具体命令在 crate 建立后由 repository
CI source of truth 定义；蓝图不预埋与尚未存在的 workspace layout 不一致的命令。发布测试还必须检查
manifest、MCP config 与 hook config
都指向已打包的同一个相对 binary path。

### 10.6 P0 release gates

以下任一情形存在时不得发布：

- 正常 Skill 或 Composition 路径指示 subagent 调用任何 Core MCP、直接访问 `.mobius/`，或把 result
  envelope 自动映射为状态；
- 任一 Mobius delegation envelope 缺少“任何 Mobius Core MCP 均禁止”或“`.mobius/` 读写均禁止”中的任一项；
- Model Core 与 Subagent Skill 存在 import、共享 schema、共享 lifecycle，或通用 Subagent package 包含
  downstream-specific API、path 或 schema knowledge；
- Trail 之外存在权威业务状态或无法 replay 的完成事实；
- CSV、session/run path、view pointer 或人工编辑能成为 transition、Evidence、Decision、proof 或 completion 输入；
- `interaction.md` 能成为业务事实、Map recovery、Evidence、Decision、proof 或 completion，或由 subagent/
  非 Route-design main Agent 作为输入读取；
- 同一 project 的 `MobiusSQLite` 数量不等于一，按 Objective/agent/功能拆库，或数据库位于其 canonical root 的 `.mobius/` 之外；
- 同一 Objective 有两个 current Stage/Attempt，或 project 有两个 active Objective；
- `InstallMap` 接受不满足模型第 3.4、3.5 节任一结构约束的 Map，包括不完整的 `Contract_μ`；
- Evidence 未满足接纳前态、Context 或 freeze 条件；
- `SealAttempt` 接受调用者自选 Packet，而不是由 Core 在同一接纳前态唯一物化；
- Packet 可以遗漏当前 Context 的已接纳 Evidence；
- 正常路径或 maintenance 对已提交 `trail_events` 执行 `UPDATE`/`DELETE`，或 projection 无法从 Trail 重建；
- Skills 把无界 Trail、完整 Ω、`SELECT *` 或 database dump 作为正常读取；正式 Wait 缺少同 snapshot
  count/bytes admission、全量或零 payload 语义、全部 admitted matching Evidence 或机械一致性检查；
- MCP/CLI 重新暴露通用 read、SQL proxy、object/event/artifact read、cursor/chunk 或兼容 facade；
- Subagent result 缺少有限预算、静默截断 correctness-critical 结果，或 fanout 重复注入相同材料与综合；
- stale、矛盾、未覆盖或模型外 Judgment 可以产生 proof；
- Objective 在 `AllCurrent` 为假时进入 Achieved；
- terminal state 接受后续业务 transition；
- `routeStatus=rejected` 能从 `Decision(replace)` 与 `CheckWait(new_route)` 之外的输入或事实派生；
- referenced artifact 与审查材料不是同一冻结内容；
- audit、doctor、report 或 hook adapter 暴露业务 mutation subcommand；
- human-gated transition 能在缺少明确用户确认，或 typed confirmation 未精确绑定 action、payload 与两级 heads 时通过；
- Driver 使用非 host-native Subagent 路径、第二 Agent Runtime，把 Runtime identity attestation 作为可用前提，
  或吞掉 spawn、配置、Runtime、权限错误并返回 success-shaped result；
- crash 可以通过正常路径产生已提交但从未 durable 的 artifact reference；
- report 失败、stale 或人工修改能改变、回滚或伪造业务 transition；
- 默认 Agent-facing SQL、mutation、skill 或 Subagent result 注入 CSV 内容、refresh task 或生成日志；或
  activation/revision 之外的 mutation 返回 interaction view path；
- Core runtime 依赖 Python、virtualenv、pip、sidecar 或第二个 Mobius executable；或插件捆绑/下载 SQLite CLI。

### 10.7 非目标

- 不兼容、导入或迁移任何既有实现或 ledger；
- 不兼容旧 session/run 路径或 CSV schema，不把 CSV 重新升级为 ledger，也不提供 CSV import/edit round-trip；
- 不包装、嵌入或调用既有 Python runtime，也不提供 Python/Rust 双栈期；
- 不做 project move/rebind；
- 不做 external immutable resolver；
- 不做云同步、多机复制、远程 reviewer transport 或 hosted service；
- 不建立自定义密码学 attestation、authority codec、共享 secret 或签名协议；
- 不为 main/subagent 建立 caller 身份认证、per-thread mutation capability、专用 sandbox 或工具隐藏层；
- 不建立 subagent ledger、任务队列、scheduler、registry、heartbeat 或 memory；
- 不把 SQLite 设计成通用 workflow engine；
- 不保存模型私有思维链或完整会话；
- 不持久化 activation/revision 前的逐轮草稿，不为 interaction 增加 Core state、Trail object、SQLite table、
  MCP tool、CLI mode、索引、签名、恢复协议或专用 eval 平台；
- 不允许一个 project 同时运行多个 active Objective；
- 不把 presentation `Run` 升级为模型对象、Objective identity 或 Attempt 别名；
- 不实现 session/run view generation 的自动 pruning 或 GC；`views/` 只按需整体重建；
- 不把 `SKILL.md`、manifest、MCP/hook 配置、SQLite 或 artifact 数据误称为第二个 runtime；
- 不为假设中的兼容性保留 fallback、alias、shim 或第二套正常路径。
